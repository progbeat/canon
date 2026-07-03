use super::model_fallback::write_model_fallback_events;
use crate::check::core::{InterrogationAnswer, InterrogationResult, ResolvedExpectation};
use crate::check::interrogation::dynamic_tool::{
    canon_show_dynamic_tools, CanonShowDynamicToolHandler,
};
use crate::check::interrogation::state::{
    evaluator_prerender_thread_reuse_key, evaluator_rendered_thread_reuse_key, CheckRuntime,
    InterrogationRunState, PrerenderEvaluatorThreadReuseKeyContext,
    RenderedEvaluatorThreadReuseKeyContext,
};
use crate::check::interrogation::{
    finalize_interrogation_answer, interrogation_result_from_answer, InterrogationRequestKind,
};
use crate::check::{evaluator_response_output_schema_for_scope, EvaluatorResponseSchemaScope};
use crate::config_types::{AgentConfig, AGAINST_TREE_DIFF_FROM, DEFAULT_DIFF_FROM};
use crate::evaluator::{
    ask_once as ask_evaluator_once, developer_instructions, effective_thinking,
    evaluator_base_instructions, evaluator_turn_prompt, is_context_window_failure,
    q_scope_is_full_project, session_failure_invalidates_thread, write_thread_lifecycle_event,
    write_thread_restart_event, BaseInstructionsContext, DeveloperInstructionsContext,
    EvaluatorError, EvaluatorRunner, EvaluatorTurnContext, EvaluatorTurnPromptContext,
    ParsedTurnResponse, PromptTemplateArtifactDir, ThreadLifecycleLog, ThreadReuseLogContext,
};
use crate::logs::DiagnosticLogWriter;
use crate::scope::{effective_ignore_patterns, sanitize_scope};
use crate::xpec_state::{LastResult, XpecStateCache};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct ThreadTurnRequest<'a> {
    pub(crate) agent: &'a AgentConfig,
    pub(crate) enforced_scope: &'a [String],
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
    pub(crate) response_contract: ThreadTurnResponseContract,
    pub(crate) request_kind: InterrogationRequestKind,
    pub(crate) expectation_id: Option<&'a str>,
    pub(crate) short_id: &'a str,
    pub(crate) question_context: &'a str,
    pub(crate) diff_from_tree_oid: &'a str,
    pub(crate) prompt: &'a str,
    pub(crate) template_artifact_dir: PromptTemplateArtifactDir,
    pub(crate) template_artifact_paths: &'a [PathBuf],
    pub(crate) last_pass: Option<&'a LastResult>,
    pub(crate) progress: Option<&'a crate::evaluator::EvaluatorProgress>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThreadTurnResponseContract {
    ExpectationResult,
    AdHocQuestion,
}

impl ThreadTurnResponseContract {
    fn for_expectation(expectation: &ResolvedExpectation) -> ThreadTurnResponseContract {
        if expectation.expected_answer.is_empty() {
            ThreadTurnResponseContract::AdHocQuestion
        } else {
            ThreadTurnResponseContract::ExpectationResult
        }
    }

    fn schema_scope(
        self,
        runtime: &CheckRuntime<'_>,
        enforced_scope: &[String],
    ) -> EvaluatorResponseSchemaScope {
        if runtime.evaluator_interrogations_never_hide_files() {
            return EvaluatorResponseSchemaScope::WithoutQuestionScopeSuggestion;
        }
        EvaluatorResponseSchemaScope::for_scope_with_question_scope_suggestion(enforced_scope)
    }
}

pub(crate) struct ResolvedDiffFrom<'a> {
    pub(crate) tree_oid: String,
    pub(crate) last_pass: Option<&'a LastResult>,
}

struct ThreadSessionSelection {
    lifecycle_log: ThreadLifecycleLog,
    reused_existing_session: bool,
}

pub(crate) fn ask_with_reused_thread<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    xpec_state: &mut XpecStateCache,
    request: ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    request
        .request_kind
        .record_started_progress_marker(request.progress);
    let current_visible_tree_oid = runtime
        .visible_tree_oid(
            &mut state.visible_tree_oid_cache,
            request.agent,
            request.enforced_scope,
        )
        .map_err(EvaluatorError::message)?;
    let reuse_visible_tree_oid = request
        .last_pass
        .and_then(|last_pass| last_pass.visible_tree_oid.as_deref())
        .unwrap_or("");
    let reuse_context = thread_reuse_log_context(runtime, &request, reuse_visible_tree_oid)?;
    let session_key =
        evaluator_prerender_thread_reuse_key(PrerenderEvaluatorThreadReuseKeyContext {
            agent: request.agent,
            scope: request.enforced_scope,
            model: request.model,
            thinking: request.thinking,
            visible_tree_oid: reuse_visible_tree_oid,
            question_context: request.question_context,
            diff_base_tree_oid: request.diff_from_tree_oid,
            checked_tree_oid: runtime.checked_tree_oid(),
        })
        .map_err(EvaluatorError::message)?;
    // A restricted retry, q-scope verification, or different rendered diff
    // transcript misses this pool and starts a separate evaluator thread.
    let existing_session = state
        .thread_sessions_by_prerender_key
        .get(&session_key)
        .cloned()
        .filter(|session_id| {
            !state.session_has_seen_dynamic_show_expectation(session_id, request.expectation_id)
        });
    let selection = match existing_session {
        Some(existing) => ThreadSessionSelection {
            lifecycle_log: thread_reuse_log(state, existing, reuse_context.clone())?,
            reused_existing_session: true,
        },
        None => start_or_reuse_thread_session_after_rendering(
            runtime,
            runner,
            state,
            &session_key,
            &current_visible_tree_oid,
            reuse_context.clone(),
            &request,
        )?,
    };
    let mut session_id = selection.lifecycle_log.session_id.clone();
    // Runtime logs expose the effective instructions for every thread start or
    // reuse, so thread behavior can be audited from the log without reading
    // derived state.
    write_thread_lifecycle_event(
        diagnostic_log,
        &selection.lifecycle_log,
        request.enforced_scope,
        request.model,
        request.thinking,
    );
    let response = match ask_current_session(
        runtime,
        runner,
        &session_id,
        state,
        diagnostic_log,
        xpec_state,
        &request,
    ) {
        Ok(response) => response,
        Err(err)
            if err.kind() == Some(crate::evaluator::EvaluatorFailureKind::ShortIdResponse)
                && state.session_has_valid_response(&session_id) =>
        {
            if let Some(progress) = request.progress {
                // Canon `↻`: a fresh-thread retry after a short-ID response
                // error started.
                progress.record_fresh_thread_retry_after_short_id_response_error_started();
            }
            clear_thread_sessions_after_failure(state);
            write_thread_restart_event(
                diagnostic_log,
                &selection.lifecycle_log,
                request.expectation_id,
                request.enforced_scope,
                request.model,
                err.message_str(),
            );
            let selection = start_or_reuse_thread_session_after_rendering(
                runtime,
                runner,
                state,
                &session_key,
                &current_visible_tree_oid,
                reuse_context.clone(),
                &request,
            )?;
            session_id = selection.lifecycle_log.session_id.clone();
            write_thread_lifecycle_event(
                diagnostic_log,
                &selection.lifecycle_log,
                request.enforced_scope,
                request.model,
                request.thinking,
            );
            match ask_current_session(
                runtime,
                runner,
                &session_id,
                state,
                diagnostic_log,
                xpec_state,
                &request,
            ) {
                Ok(response) => response,
                Err(err) => return fail_after_session_error(state, err),
            }
        }
        Err(err) if selection.reused_existing_session && is_context_window_failure(&err) => {
            if let Some(progress) = request.progress {
                // The retry is logged through the model-fallback event path
                // below, so the public timeline uses the same canon `⇄`
                // marker before the fresh thread can make another request.
                progress.record_model_fallback_started();
            }
            clear_thread_sessions_after_failure(state);
            write_model_fallback_events(
                diagnostic_log,
                request.expectation_id,
                request.model,
                None,
                err.message_str(),
            )
            .map_err(|err| EvaluatorError::message(err.to_string()))?;
            write_thread_restart_event(
                diagnostic_log,
                &selection.lifecycle_log,
                request.expectation_id,
                request.enforced_scope,
                request.model,
                err.message_str(),
            );
            let selection = start_or_reuse_thread_session_after_rendering(
                runtime,
                runner,
                state,
                &session_key,
                &current_visible_tree_oid,
                reuse_context.clone(),
                &request,
            )?;
            session_id = selection.lifecycle_log.session_id.clone();
            write_thread_lifecycle_event(
                diagnostic_log,
                &selection.lifecycle_log,
                request.enforced_scope,
                request.model,
                request.thinking,
            );
            match ask_current_session(
                runtime,
                runner,
                &session_id,
                state,
                diagnostic_log,
                xpec_state,
                &request,
            ) {
                Ok(response) => response,
                Err(err) => return fail_after_session_error(state, err),
            }
        }
        Err(err) => return fail_after_session_error(state, err),
    };
    if !retire_thread_sessions_after_turn(state, runner.take_retired_sessions(), &session_id) {
        state
            .thread_sessions_by_prerender_key
            .insert(session_key, session_id);
    }
    Ok(response)
}

fn ask_current_session<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    session_id: &str,
    state: &mut InterrogationRunState,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    xpec_state: &mut XpecStateCache,
    request: &ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let Some(session_root) = state.session_roots_by_id.get(session_id).cloned() else {
        return Err(EvaluatorError::message(format!(
            "missing evaluator session root for session {}",
            session_id
        )));
    };
    if let Err(err) = state.activate_session_root(session_id) {
        state.clear_thread_sessions();
        return Err(EvaluatorError::message(err));
    }
    ask_in_thread(
        runtime,
        runner,
        session_id,
        session_root.as_path(),
        state,
        diagnostic_log,
        xpec_state,
        request,
    )
}

#[allow(clippy::too_many_arguments)]
fn ask_in_thread<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    session_id: &str,
    session_root: &Path,
    state: &mut InterrogationRunState,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    xpec_state: &mut XpecStateCache,
    request: &ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let turn = EvaluatorTurnContext {
        session_id,
        model: request.model,
        thinking: request.thinking,
    };
    let visible_scope = runtime
        .visible_scope(request.agent, request.enforced_scope)
        .map_err(EvaluatorError::message)?;
    let schema_scope = request
        .response_contract
        .schema_scope(runtime, request.enforced_scope);
    let output_schema = evaluator_response_output_schema_for_scope(schema_scope, request.short_id);
    let answered_short_ids = state.answered_short_ids_for_session(session_id);
    // This is one turn/start request. `ThreadTurnRequest::request_kind`
    // classifies whether it is an initial request or a non-initial follow-up,
    // and `ask_with_reused_thread` records any request-start timeline marker
    // before session setup can emit thread/start.
    let (response, shown_expectation_ids) = {
        let parse_cache = &mut state.parse_cache;
        if request.expectation_id.is_some() {
            let mut dynamic_tool_handler = CanonShowDynamicToolHandler::new(
                runtime,
                request.expectation_id,
                xpec_state,
                &mut state.visible_tree_oid_cache,
            );
            let response = ask_evaluator_once(
                runner,
                &turn,
                request.prompt,
                request.agent,
                schema_scope,
                &output_schema,
                request.short_id,
                &answered_short_ids,
                &visible_scope,
                session_root,
                parse_cache,
                diagnostic_log,
                request.expectation_id,
                Some(&mut dynamic_tool_handler),
            );
            let shown_expectation_ids = dynamic_tool_handler.into_shown_expectation_ids();
            (response, shown_expectation_ids)
        } else {
            (
                ask_evaluator_once(
                    runner,
                    &turn,
                    request.prompt,
                    request.agent,
                    schema_scope,
                    &output_schema,
                    request.short_id,
                    &answered_short_ids,
                    &visible_scope,
                    session_root,
                    parse_cache,
                    diagnostic_log,
                    request.expectation_id,
                    None,
                ),
                BTreeSet::new(),
            )
        }
    };
    state.record_session_dynamic_show_expectation_ids(session_id, shown_expectation_ids);
    let response = response?;
    if response.schema_valid {
        state.record_session_answered_short_id(session_id, request.short_id);
    }
    Ok(response)
}

fn start_or_reuse_thread_session_after_rendering<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    state: &mut InterrogationRunState,
    session_key: &str,
    visible_tree_oid: &str,
    reuse_context: ThreadReuseLogContext,
    request: &ThreadTurnRequest<'_>,
) -> Result<ThreadSessionSelection, EvaluatorError> {
    let visible_scope = runtime
        .visible_scope(request.agent, request.enforced_scope)
        .map_err(EvaluatorError::message)?;
    let ignore = effective_ignore_patterns(request.agent).map_err(EvaluatorError::message)?;
    let visible_file_count = runtime.visible_file_count(
        &mut state.visible_tree_oid_cache,
        request.agent,
        request.enforced_scope,
    )?;
    let session_root = runtime
        .session_root_for_scope(request.agent, request.enforced_scope, visible_tree_oid)
        .map_err(EvaluatorError::message)?;
    // Render the developer-instructions resource template. `question_context`
    // is the value for that template's `xpec.instructions` input slot,
    // not a second prompt or instruction template.
    let mut template_artifact_paths = request.template_artifact_paths.to_vec();
    let developer_instructions = developer_instructions(DeveloperInstructionsContext {
        root: runtime.root,
        template_artifact_dir: request.template_artifact_dir.clone(),
        template_artifact_paths: &mut template_artifact_paths,
        in_place: runtime.is_in_place(),
        diff_from_tree_oid: request.diff_from_tree_oid,
        checked_tree_oid: runtime.checked_tree_oid(),
        question_context: request.question_context,
        q_scope: request.enforced_scope,
        ignore: &ignore,
        visible_scope: &visible_scope,
        checked_file_count: runtime.checked_file_count(),
        visible_file_count,
        last_pass: request.last_pass,
    })
    .map_err(EvaluatorError::message)?;
    let base_instructions = evaluator_base_instructions(BaseInstructionsContext {
        in_place: runtime.is_in_place(),
        full_scope: q_scope_is_full_project(request.enforced_scope),
    })
    .map_err(EvaluatorError::message)?;
    let rendered_key =
        evaluator_rendered_thread_reuse_key(RenderedEvaluatorThreadReuseKeyContext {
            agent: request.agent,
            model: request.model,
            thinking: request.thinking,
            base_instructions: &base_instructions,
            developer_instructions: &developer_instructions,
        });
    if let Some(existing) = state
        .thread_sessions_by_rendered_instructions_key
        .get(&rendered_key)
        .cloned()
        .filter(|session_id| {
            !state.session_has_seen_dynamic_show_expectation(session_id, request.expectation_id)
        })
    {
        // This is the second evaluator-thread reuse lookup required by canon:
        // after rendering instructions anyway, reuse a live thread whose
        // rendered base/developer instructions are identical.
        state
            .thread_sessions_by_prerender_key
            .insert(session_key.to_string(), existing.clone());
        return Ok(ThreadSessionSelection {
            lifecycle_log: thread_reuse_log(state, existing, reuse_context)?,
            reused_existing_session: true,
        });
    }
    let session_isolation = if runtime.is_in_place() {
        // In-place mode starts the evaluator in the checked directory itself.
        // Moving that directory to an isolation path would make the evaluator
        // cwd a sandbox copy instead of the checked directory required by the
        // in-place canon.
        None
    } else {
        state
            .isolate_session_root(&session_root)
            .map_err(EvaluatorError::message)?
    };
    let session_cwd = session_isolation
        .as_ref()
        .map(|isolation| isolation.path())
        .unwrap_or(session_root.as_path());
    let dynamic_tools = if request.expectation_id.is_some() {
        canon_show_dynamic_tools()
    } else {
        Vec::new()
    };
    let created = match runner.start_session(
        session_cwd,
        &template_artifact_paths,
        &base_instructions,
        &developer_instructions,
        request.agent,
        request.model,
        request.thinking,
        request.enforced_scope,
        &dynamic_tools,
    ) {
        Ok(created) => created,
        Err(err) => return fail_after_session_error(state, err),
    };
    state
        .session_base_instructions
        .insert(created.clone(), base_instructions.clone());
    state
        .session_instructions
        .insert(created.clone(), developer_instructions.clone());
    state
        .session_roots_by_id
        .insert(created.clone(), session_cwd.to_path_buf());
    if let Some(isolation) = session_isolation {
        state.session_isolations.insert(created.clone(), isolation);
    }
    state
        .thread_sessions_by_prerender_key
        .insert(session_key.to_string(), created.clone());
    state
        .thread_sessions_by_rendered_instructions_key
        .insert(rendered_key, created.clone());
    Ok(ThreadSessionSelection {
        lifecycle_log: ThreadLifecycleLog {
            event: "thread.start",
            session_id: created,
            base_instructions,
            developer_instructions,
            reuse_context,
        },
        reused_existing_session: false,
    })
}

fn thread_reuse_log(
    state: &InterrogationRunState,
    session_id: String,
    reuse_context: ThreadReuseLogContext,
) -> Result<ThreadLifecycleLog, EvaluatorError> {
    let Some(base_instructions) = state.session_base_instructions.get(&session_id).cloned() else {
        return Err(EvaluatorError::message(format!(
            "missing base instructions for reused session {}",
            session_id
        )));
    };
    let Some(developer_instructions) = state.session_instructions.get(&session_id).cloned() else {
        return Err(EvaluatorError::message(format!(
            "missing developer instructions for reused session {}",
            session_id
        )));
    };
    Ok(ThreadLifecycleLog {
        event: "thread.reuse",
        session_id,
        base_instructions,
        developer_instructions,
        reuse_context,
    })
}

fn thread_reuse_log_context(
    runtime: &CheckRuntime<'_>,
    request: &ThreadTurnRequest<'_>,
    visible_tree_oid: &str,
) -> Result<ThreadReuseLogContext, EvaluatorError> {
    Ok(ThreadReuseLogContext {
        visible_tree_oid: visible_tree_oid.to_string(),
        diff_base_tree_oid: request.diff_from_tree_oid.to_string(),
        checked_tree_oid: runtime.checked_tree_oid().to_string(),
        turn_prompt: request.prompt.to_string(),
        question_context: request.question_context.to_string(),
        plugins: request.agent.plugins.clone(),
        ignore: effective_ignore_patterns(request.agent).map_err(EvaluatorError::message)?,
    })
}

fn clear_thread_sessions_after_failure(state: &mut InterrogationRunState) {
    // Reuse applies only to successful, still-live evaluator threads for the
    // same model and visible-tree context. Technical app-server failures can
    // retire the backing process, and short-ID response errors deliberately
    // discard the conversational context before a retry.
    state.clear_thread_sessions();
}

fn retire_thread_sessions_after_turn(
    state: &mut InterrogationRunState,
    retired_sessions: Vec<String>,
    active_session_id: &str,
) -> bool {
    if retired_sessions.is_empty() {
        return false;
    }
    let retired_sessions = retired_sessions.into_iter().collect::<BTreeSet<_>>();
    state
        .thread_sessions_by_prerender_key
        .retain(|_, session_id| !retired_sessions.contains(session_id));
    state
        .thread_sessions_by_rendered_instructions_key
        .retain(|_, session_id| !retired_sessions.contains(session_id));
    state
        .session_instructions
        .retain(|session_id, _| !retired_sessions.contains(session_id));
    state
        .session_base_instructions
        .retain(|session_id, _| !retired_sessions.contains(session_id));
    state
        .session_roots_by_id
        .retain(|session_id, _| !retired_sessions.contains(session_id));
    state
        .session_answered_short_ids
        .retain(|session_id, _| !retired_sessions.contains(session_id));
    state
        .session_dynamic_show_expectation_ids
        .retain(|session_id, _| !retired_sessions.contains(session_id));
    state
        .session_isolations
        .retain(|session_id, _| !retired_sessions.contains(session_id));
    retired_sessions.contains(active_session_id)
}

fn fail_after_session_error<T>(
    state: &mut InterrogationRunState,
    err: EvaluatorError,
) -> Result<T, EvaluatorError> {
    if session_failure_invalidates_thread(&err) {
        clear_thread_sessions_after_failure(state);
    }
    Err(err)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn interrogate_expectation_with_model<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    expectation: &ResolvedExpectation,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    xpec_state: &mut XpecStateCache,
    enforced_scope: &[String],
    model: Option<&str>,
    request_kind: InterrogationRequestKind,
    progress: Option<&crate::evaluator::EvaluatorProgress>,
) -> Result<InterrogationResult, EvaluatorError> {
    let answer = interrogate_expectation_answer_with_model(
        runtime,
        expectation,
        runner,
        diagnostic_log,
        state,
        xpec_state,
        enforced_scope,
        model,
        request_kind,
        progress,
    )?;
    interrogation_result_from_answer(expectation, diagnostic_log, answer)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn interrogate_expectation_answer_with_model<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    expectation: &ResolvedExpectation,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    xpec_state: &mut XpecStateCache,
    enforced_scope: &[String],
    model: Option<&str>,
    request_kind: InterrogationRequestKind,
    progress: Option<&crate::evaluator::EvaluatorProgress>,
) -> Result<InterrogationAnswer, EvaluatorError> {
    // Expectation checks may start from a last-pass restricted scope, but
    // after sanitization this path shares `canon ask`'s first-turn construction:
    // developer instructions and the turn prompt are rendered from
    // `resources/prompts/` plus runtime data.
    let enforced_scope = sanitize_scope(enforced_scope)?;
    let last_pass = if runtime.is_in_place() || expectation.id.is_empty() {
        None
    } else {
        xpec_state
            .read_last_pass(runtime.root, expectation)
            .map_err(EvaluatorError::message)?
    };
    ask_expectation_turn(
        runtime,
        expectation,
        runner,
        diagnostic_log,
        state,
        xpec_state,
        &enforced_scope,
        model,
        last_pass.as_ref(),
        request_kind,
        progress,
    )
}

#[allow(clippy::too_many_arguments)]
fn ask_expectation_turn<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    expectation: &ResolvedExpectation,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    xpec_state: &mut XpecStateCache,
    enforced_scope: &[String],
    model: Option<&str>,
    last_pass: Option<&LastResult>,
    request_kind: InterrogationRequestKind,
    progress: Option<&crate::evaluator::EvaluatorProgress>,
) -> Result<InterrogationAnswer, EvaluatorError> {
    let diff_from = resolve_diff_from(runtime, expectation, last_pass)?;
    let template_artifact_dir =
        PromptTemplateArtifactDir::Lazy(Arc::clone(&state.prompt_template_output_dir_cache));
    let mut template_artifact_paths = Vec::new();
    let prompt = evaluator_turn_prompt(EvaluatorTurnPromptContext {
        root: runtime.root,
        template_artifact_dir: template_artifact_dir.clone(),
        template_artifact_paths: &mut template_artifact_paths,
        short_id: &expectation.display_id,
        question: &expectation.question,
        expected_answer: &expectation.expected_answer,
        in_place: runtime.is_in_place(),
        diff_from: &expectation.diff_from,
        target: expectation.target.as_ref().map(|target| target.as_str()),
        last_pass: diff_from.last_pass,
    })
    .map_err(EvaluatorError::message)?;
    let thinking = effective_thinking(&expectation.agent, expectation);
    let response = ask_with_reused_thread(
        runtime,
        runner,
        diagnostic_log,
        state,
        xpec_state,
        ThreadTurnRequest {
            agent: &expectation.agent,
            enforced_scope,
            model,
            thinking,
            response_contract: ThreadTurnResponseContract::for_expectation(expectation),
            request_kind,
            expectation_id: (!expectation.id.is_empty()).then_some(expectation.id.as_str()),
            short_id: &expectation.display_id,
            // This is question-scoped canon config data. The implementation-owned
            // evaluator instruction source is the template in `resources/prompts/`;
            // this text is only a value embedded by that source.
            question_context: &expectation.question_context,
            diff_from_tree_oid: &diff_from.tree_oid,
            prompt: &prompt,
            template_artifact_dir,
            template_artifact_paths: &template_artifact_paths,
            last_pass: diff_from.last_pass,
            progress,
        },
    )?;
    finalize_interrogation_answer(
        runtime,
        state,
        &expectation.agent,
        enforced_scope,
        response.answer,
        response.usage,
        response.context_compacted,
    )
}

pub(crate) fn resolve_diff_from<'a>(
    runtime: &CheckRuntime<'_>,
    expectation: &ResolvedExpectation,
    last_pass: Option<&'a LastResult>,
) -> Result<ResolvedDiffFrom<'a>, EvaluatorError> {
    if runtime.is_in_place() {
        // In-place mode has no Git-backed diff base. Its resolved expectations
        // are validated before interrogation, and prompt rendering clears any
        // diff-only turn inputs for this mode.
        return Ok(ResolvedDiffFrom {
            tree_oid: runtime.checked_tree_oid().to_string(),
            last_pass: None,
        });
    }
    // This is the canon `diff-from` resolver for prompt-rendered Git diffs.
    // `:checkpoint` uses the last pass checked tree only while that tree object
    // exists in the repository, otherwise it falls back to the run's against
    // tree. `:against-tree` uses the against tree directly. Other values use
    // the same TreeSource resolution as check command `<TREE>` options.
    let diff_from = expectation.diff_from.as_str();
    if diff_from == DEFAULT_DIFF_FROM {
        return checkpoint_diff_base(runtime.root, last_pass, runtime.against_tree_oid());
    }
    if diff_from == AGAINST_TREE_DIFF_FROM {
        return Ok(ResolvedDiffFrom {
            tree_oid: runtime.against_tree_oid().to_string(),
            last_pass,
        });
    }
    Ok(ResolvedDiffFrom {
        tree_oid: explicit_diff_base_tree_oid(runtime.root, diff_from)?,
        last_pass,
    })
}

fn checkpoint_diff_base<'a>(
    root: &Path,
    last_pass: Option<&'a LastResult>,
    against_tree_oid: &str,
) -> Result<ResolvedDiffFrom<'a>, EvaluatorError> {
    if let Some(checked_tree_oid) =
        last_pass.and_then(|last_pass| last_pass.checked_tree_oid.as_deref())
    {
        if crate::git::git_object_oid_has_known_shape(checked_tree_oid)
            && crate::git::tree_object_exists(root, checked_tree_oid)
                .map_err(EvaluatorError::message)?
        {
            return Ok(ResolvedDiffFrom {
                tree_oid: checked_tree_oid.to_string(),
                last_pass,
            });
        }
    }
    Ok(ResolvedDiffFrom {
        tree_oid: against_tree_oid.to_string(),
        last_pass: None,
    })
}

fn explicit_diff_base_tree_oid(root: &Path, tree: &str) -> Result<String, EvaluatorError> {
    crate::git::TreeSource::resolve(root, tree, "diff-from")
        .and_then(|source| source.tree_oid_for_prompt_diff(root))
        .map_err(EvaluatorError::message)
}

#[cfg(test)]
mod tests {
    use super::checkpoint_diff_base;
    use crate::xpec_state::{LastResult, LastResultStatus};
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn checkpoint_diff_base_uses_existing_checkpoint_tree() {
        let root = git_project("checkpoint-existing");
        let checked_tree_oid = crate::git::staged_tree_oid(&root).unwrap();
        let last_pass = last_pass_with_checked_tree_oid(&checked_tree_oid);

        let resolved = checkpoint_diff_base(&root, Some(&last_pass), "against-tree").unwrap();

        assert_eq!(resolved.tree_oid, checked_tree_oid);
        assert!(resolved.last_pass.is_some());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn checkpoint_diff_base_ignores_missing_checkpoint_tree() {
        let root = git_project("checkpoint-missing");
        let missing_tree_oid = "ffffffffffffffffffffffffffffffffffffffff";
        let last_pass = last_pass_with_checked_tree_oid(missing_tree_oid);

        let resolved = checkpoint_diff_base(&root, Some(&last_pass), "against-tree").unwrap();

        assert_eq!(resolved.tree_oid, "against-tree");
        assert!(resolved.last_pass.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn checkpoint_diff_base_ignores_non_oid_checkpoint_tree() {
        let root = git_project("checkpoint-revspec");
        let last_pass = last_pass_with_checked_tree_oid("HEAD^{tree}");

        let resolved = checkpoint_diff_base(&root, Some(&last_pass), "against-tree").unwrap();

        assert_eq!(resolved.tree_oid, "against-tree");
        assert!(resolved.last_pass.is_none());
        let _ = fs::remove_dir_all(root);
    }

    fn last_pass_with_checked_tree_oid(checked_tree_oid: &str) -> LastResult {
        LastResult {
            response_timestamp: "1970-01-01T00:00:00Z".to_string(),
            updated_timestamp: "1970-01-01T00:00:00Z".to_string(),
            status: LastResultStatus::Pass,
            response: json!({
                "answer": "yes",
                "evidence": "`src/main.rs`",
                "qScopeSuggestion": ["."]
            }),
            q_scope: vec![".".to_string()],
            visible_scope: vec![".".to_string()],
            checked_tree_oid: Some(checked_tree_oid.to_string()),
            visible_tree_oid: None,
        }
    }

    fn git_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("canon-{name}-{}-{unique}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        git(&root, &["init"]);
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        git(&root, &["add", "src/main.rs"]);
        root
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
