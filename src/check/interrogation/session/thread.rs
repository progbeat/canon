use super::model_fallback::write_model_fallback_events;
use crate::check::core::{InterrogationResult, SelectedExpectation};
use crate::check::interrogation::finalize_interrogation_response;
use crate::check::interrogation::state::{
    evaluator_thread_reuse_key, CheckRuntime, InterrogationRunState,
};
use crate::config_types::{AgentConfig, AGAINST_TREE_DIFF_FROM, DEFAULT_DIFF_FROM};
use crate::evaluator::{
    ask_once as ask_evaluator_once, create_prompt_template_output_dir, developer_instructions,
    effective_thinking, evaluator_turn_prompt, is_context_window_failure,
    session_failure_invalidates_thread, write_thread_lifecycle_event, write_thread_restart_event,
    DeveloperInstructionsContext, EvaluatorError, EvaluatorResponseParseCache, EvaluatorRunner,
    EvaluatorTurnContext, ParsedTurnResponse, ThreadLifecycleLog,
};
use crate::logs::DiagnosticLogWriter;
use crate::scope::{sanitize_scope, visible_scope};
use crate::xpec_state::{LastResult, XpecStateCache};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Clone, Copy)]
pub(crate) struct ThreadTurnRequest<'a> {
    pub(crate) agent: &'a AgentConfig,
    pub(crate) enforced_scope: &'a [String],
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
    pub(crate) expectation_id: Option<&'a str>,
    pub(crate) expectation_instructions: &'a str,
    pub(crate) diff_from_tree_oid: &'a str,
    pub(crate) prompt: &'a str,
    pub(crate) template_output_dir: &'a Path,
    pub(crate) last_pass: Option<&'a LastResult>,
}

pub(crate) fn ask_with_reused_thread<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    request: ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let current_visible_tree_oid = state
        .visible_tree_oid_cache
        .visible_tree_oid(
            runtime.root,
            runtime.tree_source,
            request.agent,
            request.enforced_scope,
        )
        .map_err(EvaluatorError::message)?;
    let reuse_visible_tree_oid = request
        .last_pass
        .and_then(|last_pass| last_pass.visible_tree_oid.as_deref())
        .unwrap_or("");
    let session_key = evaluator_thread_reuse_key(
        request.agent,
        request.enforced_scope,
        request.model,
        reuse_visible_tree_oid,
        request.expectation_instructions,
        request.diff_from_tree_oid,
        &runtime.tree_context.checked_tree_oid,
    )
    .map_err(EvaluatorError::message)?;
    // A restricted retry, q-scope verification, or different rendered diff
    // transcript misses this pool and starts a separate evaluator thread.
    let existing_session = state
        .thread_sessions_by_reuse_key
        .get(&session_key)
        .cloned();
    let had_existing_session = existing_session.is_some();
    let lifecycle_log = match existing_session {
        Some(existing) => thread_reuse_log(state, existing)?,
        None => start_thread_session(
            runtime,
            runner,
            state,
            &session_key,
            &current_visible_tree_oid,
            request,
        )?,
    };
    let mut session_id = lifecycle_log.session_id.clone();
    // Runtime logs expose the effective instructions for every thread start or
    // reuse, so thread behavior can be audited from the log without reading
    // derived state.
    write_thread_lifecycle_event(
        diagnostic_log,
        &lifecycle_log,
        request.enforced_scope,
        request.model,
        request.thinking,
    );
    let response = match ask_current_session(runner, &session_id, state, diagnostic_log, request) {
        Ok(response) => response,
        Err(err) if had_existing_session && is_context_window_failure(&err) => {
            clear_thread_sessions_after_failure(state);
            write_model_fallback_events(
                diagnostic_log,
                request.expectation_id,
                request.model,
                None,
                err.message_str(),
            );
            write_thread_restart_event(
                diagnostic_log,
                &session_id,
                request.expectation_id,
                request.enforced_scope,
                request.model,
                &lifecycle_log.developer_instructions,
                err.message_str(),
            );
            let lifecycle_log = start_thread_session(
                runtime,
                runner,
                state,
                &session_key,
                &current_visible_tree_oid,
                request,
            )?;
            session_id = lifecycle_log.session_id.clone();
            write_thread_lifecycle_event(
                diagnostic_log,
                &lifecycle_log,
                request.enforced_scope,
                request.model,
                request.thinking,
            );
            match ask_current_session(runner, &session_id, state, diagnostic_log, request) {
                Ok(response) => response,
                Err(err) => return fail_after_session_error(state, err),
            }
        }
        Err(err) => return fail_after_session_error(state, err),
    };
    if !retire_thread_sessions_after_turn(state, runner.take_retired_sessions(), &session_id) {
        state
            .thread_sessions_by_reuse_key
            .insert(session_key, session_id);
    }
    Ok(response)
}

fn ask_current_session<R: EvaluatorRunner>(
    runner: &mut R,
    session_id: &str,
    state: &mut InterrogationRunState,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    request: ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let Some(session_root) = state.session_roots_by_id.get(session_id).cloned() else {
        return Err(EvaluatorError::message(format!(
            "missing evaluator session root for session {}",
            session_id
        )));
    };
    ask_in_thread(
        runner,
        session_id,
        request.agent,
        session_root.as_path(),
        &mut state.parse_cache,
        diagnostic_log,
        request,
    )
}

fn ask_in_thread<R: EvaluatorRunner>(
    runner: &mut R,
    session_id: &str,
    agent: &AgentConfig,
    session_root: &Path,
    parse_cache: &mut EvaluatorResponseParseCache,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    request: ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let turn = EvaluatorTurnContext {
        session_id,
        model: request.model,
        thinking: request.thinking,
    };
    let visible_scope =
        visible_scope(agent, request.enforced_scope).map_err(EvaluatorError::message)?;
    // The evaluator turn boundary owns agent.request/agent.response runtime
    // log events for the initial turn and any repair turn.
    ask_evaluator_once(
        runner,
        &turn,
        request.prompt,
        agent,
        &visible_scope,
        session_root,
        parse_cache,
        diagnostic_log,
        request.expectation_id,
    )
}

fn start_thread_session<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    state: &mut InterrogationRunState,
    session_key: &str,
    visible_tree_oid: &str,
    request: ThreadTurnRequest<'_>,
) -> Result<ThreadLifecycleLog, EvaluatorError> {
    let visible_scope =
        visible_scope(request.agent, request.enforced_scope).map_err(EvaluatorError::message)?;
    let visible_file_count = runtime.visible_file_count(
        &mut state.visible_tree_oid_cache,
        request.agent,
        request.enforced_scope,
    )?;
    let session_root = runtime
        .session_root_for_scope(request.agent, request.enforced_scope, visible_tree_oid)
        .map_err(EvaluatorError::message)?;
    let session_isolation = state
        .isolate_session_root(&session_root)
        .map_err(EvaluatorError::message)?;
    let session_cwd = session_isolation
        .as_ref()
        .map(|isolation| isolation.path())
        .unwrap_or(session_root.as_path());
    let developer_instructions = developer_instructions(DeveloperInstructionsContext {
        root: runtime.root,
        template_output_dir: request.template_output_dir,
        diff_from_tree_oid: request.diff_from_tree_oid,
        checked_tree_oid: &runtime.tree_context.checked_tree_oid,
        expectation_instructions: request.expectation_instructions,
        visible_scope: &visible_scope,
        checked_file_count: runtime.tree_context.checked_file_count,
        visible_file_count,
        last_pass: request.last_pass,
    })
    .map_err(EvaluatorError::message)?;
    let created = match runner.start_session(
        session_cwd,
        request.template_output_dir,
        &developer_instructions,
        request.agent,
        request.model,
        request.thinking,
        request.enforced_scope,
    ) {
        Ok(created) => created,
        Err(err) => return fail_after_session_error(state, err),
    };
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
        .thread_sessions_by_reuse_key
        .insert(session_key.to_string(), created.clone());
    Ok(ThreadLifecycleLog {
        event: "thread.start",
        session_id: created,
        developer_instructions,
    })
}

fn thread_reuse_log(
    state: &InterrogationRunState,
    session_id: String,
) -> Result<ThreadLifecycleLog, EvaluatorError> {
    let Some(developer_instructions) = state.session_instructions.get(&session_id).cloned() else {
        return Err(EvaluatorError::message(format!(
            "missing developer instructions for reused session {}",
            session_id
        )));
    };
    Ok(ThreadLifecycleLog {
        event: "thread.reuse",
        session_id,
        developer_instructions,
    })
}

fn clear_thread_sessions_after_failure(state: &mut InterrogationRunState) {
    // Reuse applies only to successful, still-live evaluator threads for the
    // same model and visible-tree context. Technical app-server failures can
    // retire the backing process, so keeping the old session ID would point at
    // a stale or missing thread rather than preserving the same Codex thread.
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
        .thread_sessions_by_reuse_key
        .retain(|_, session_id| !retired_sessions.contains(session_id));
    state
        .session_instructions
        .retain(|session_id, _| !retired_sessions.contains(session_id));
    state
        .session_roots_by_id
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
    expectation: &SelectedExpectation,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    xpec_state: &mut XpecStateCache,
    enforced_scope: &[String],
    model: Option<&str>,
) -> Result<InterrogationResult, EvaluatorError> {
    // Expectation mode may start from a last-pass restricted scope, but
    // after sanitization this path shares query mode's first-turn construction:
    // developer instructions and the turn prompt are rendered from
    // `resources/prompts/` plus runtime data.
    let enforced_scope = sanitize_scope(enforced_scope)?;
    let last_pass = xpec_state
        .read_last_pass(runtime.root, expectation)
        .map_err(EvaluatorError::message)?;
    ask_expectation_turn(
        runtime,
        expectation,
        runner,
        diagnostic_log,
        state,
        &enforced_scope,
        model,
        last_pass.as_ref(),
    )
}

#[allow(clippy::too_many_arguments)]
fn ask_expectation_turn<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    expectation: &SelectedExpectation,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    enforced_scope: &[String],
    model: Option<&str>,
    last_pass: Option<&LastResult>,
) -> Result<InterrogationResult, EvaluatorError> {
    let template_output_dir =
        create_prompt_template_output_dir().map_err(EvaluatorError::message)?;
    let diff_from_tree_oid = resolve_diff_from_tree_oid(runtime, expectation, last_pass)?;
    let prompt = evaluator_turn_prompt(
        runtime.root,
        &template_output_dir,
        &expectation.question,
        &expectation.expected_answer,
        &expectation.diff_from,
        expectation.target.as_ref().map(|target| target.as_str()),
        last_pass,
    )
    .map_err(EvaluatorError::message)?;
    let thinking = effective_thinking(&expectation.agent, expectation);
    let response = ask_with_reused_thread(
        runtime,
        runner,
        diagnostic_log,
        state,
        ThreadTurnRequest {
            agent: &expectation.agent,
            enforced_scope,
            model,
            thinking,
            expectation_id: Some(&expectation.id),
            expectation_instructions: &expectation.instructions,
            diff_from_tree_oid: &diff_from_tree_oid,
            prompt: &prompt,
            template_output_dir: &template_output_dir,
            last_pass,
        },
    )?;
    finalize_interrogation_response(
        runtime,
        expectation,
        diagnostic_log,
        state,
        enforced_scope,
        response,
    )
}

pub(crate) fn resolve_diff_from_tree_oid(
    runtime: &CheckRuntime<'_>,
    expectation: &SelectedExpectation,
    last_pass: Option<&LastResult>,
) -> Result<String, EvaluatorError> {
    // Canon `diff-from` resolution: `:checkpoint` prefers the last pass
    // checked tree and falls back to the against tree; `:against-tree` uses
    // the against tree; any other value is resolved as a tree source.
    match expectation.diff_from.as_str() {
        DEFAULT_DIFF_FROM => Ok(checkpoint_diff_base_tree_oid(
            last_pass,
            &runtime.tree_context.against_tree_oid,
        )
        .to_string()),
        AGAINST_TREE_DIFF_FROM => Ok(runtime.tree_context.against_tree_oid.clone()),
        tree => explicit_diff_base_tree_oid(runtime.root, tree),
    }
}

fn checkpoint_diff_base_tree_oid<'a>(
    last_pass: Option<&'a LastResult>,
    against_tree_oid: &'a str,
) -> &'a str {
    last_pass
        .and_then(|last_pass| last_pass.checked_tree_oid.as_deref())
        .unwrap_or(against_tree_oid)
}

fn explicit_diff_base_tree_oid(root: &Path, tree: &str) -> Result<String, EvaluatorError> {
    crate::git::TreeSource::resolve(root, tree, "diff-from")
        .and_then(|source| source.tree_oid_for_prompt_diff(root))
        .map_err(EvaluatorError::message)
}
