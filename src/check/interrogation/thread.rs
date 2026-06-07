use crate::check::core::types::{InterrogationResult, SelectedExpectation};
use crate::check::interrogation::model_fallback::write_model_fallback_events;
use crate::check::interrogation::records::finalize_interrogation_response;
use crate::check::interrogation::state::{
    evaluator_thread_reuse_key, CheckRuntime, InterrogationRunState,
};
use crate::config_types::AgentConfig;
use crate::evaluator::{
    ask_once, developer_instructions, effective_thinking, evaluator_turn_prompt,
    is_context_window_failure, session_failure_invalidates_thread, write_thread_lifecycle_event,
    write_thread_restart_event, DeveloperInstructionsContext, EvaluatorError,
    EvaluatorResponseParseCache, EvaluatorRunner, EvaluatorTurnContext, ParsedTurnResponse,
    ThreadLifecycleLog,
};
use crate::history::{against_tree_answer_with_cache, HistoryCache};
use crate::logs::DiagnosticLogWriter;
use crate::scope::{sanitize_scope, visible_scope};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Clone, Copy)]
pub(crate) struct ThreadTurnRequest<'a> {
    pub(crate) agent: &'a AgentConfig,
    pub(crate) enforced_scope: &'a [String],
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
    pub(crate) expectation_id: Option<&'a str>,
    pub(crate) prompt: &'a str,
}

pub(crate) fn ask_with_reused_thread<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    request: ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let visible_tree_oid = state
        .visible_tree_oid_cache
        .visible_tree_oid(
            runtime.root,
            runtime.tree_source,
            request.agent,
            request.enforced_scope,
        )
        .map_err(EvaluatorError::message)?;
    let session_key = evaluator_thread_reuse_key(
        request.agent,
        request.enforced_scope,
        request.model,
        &visible_tree_oid,
    )
    .map_err(EvaluatorError::message)?;
    // The lookup key begins with the evaluator model and visibleTreeOid. A
    // restricted retry or q-scope verification with a different visible tree
    // therefore misses this pool and starts a separate evaluator thread.
    let existing_session = state
        .thread_sessions_by_reuse_key
        .get(&session_key)
        .cloned();
    let had_existing_session = existing_session.is_some();
    let lifecycle_log = match existing_session {
        Some(existing) => thread_reuse_log(state, existing, request),
        None => start_thread_session(
            runtime,
            runner,
            state,
            &session_key,
            &visible_tree_oid,
            request,
        )?,
    };
    let mut session_id = lifecycle_log.session_id.clone();
    write_thread_lifecycle_event(
        diagnostic_log,
        &lifecycle_log,
        request.enforced_scope,
        request.model,
        request.thinking,
    )?;
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
            )?;
            write_thread_restart_event(
                diagnostic_log,
                &session_id,
                request.expectation_id,
                request.enforced_scope,
                request.model,
                &lifecycle_log.developer_instructions,
                err.message_str(),
            )?;
            let lifecycle_log = start_thread_session(
                runtime,
                runner,
                state,
                &session_key,
                &visible_tree_oid,
                request,
            )?;
            session_id = lifecycle_log.session_id.clone();
            write_thread_lifecycle_event(
                diagnostic_log,
                &lifecycle_log,
                request.enforced_scope,
                request.model,
                request.thinking,
            )?;
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
    let session_root = state.session_roots_by_id.get(session_id).cloned();
    ask_in_thread(
        runner,
        session_id,
        request.agent,
        session_root.as_deref(),
        &mut state.parse_cache,
        diagnostic_log,
        request,
    )
}

fn ask_in_thread<R: EvaluatorRunner>(
    runner: &mut R,
    session_id: &str,
    agent: &AgentConfig,
    session_root: Option<&Path>,
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
    ask_once(
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
    let developer_instructions = developer_instructions(DeveloperInstructionsContext {
        root: runtime.root,
        against_tree_oid: &runtime.tree_context.against_tree_oid,
        checked_tree_oid: &runtime.tree_context.checked_tree_oid,
        visible_scope: &visible_scope,
        checked_file_count: runtime.tree_context.checked_file_count,
        visible_file_count,
    })
    .map_err(EvaluatorError::message)?;
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
    let created = match runner.start_session(
        session_cwd,
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
    _request: ThreadTurnRequest<'_>,
) -> ThreadLifecycleLog {
    let developer_instructions = state
        .session_instructions
        .get(&session_id)
        .cloned()
        .unwrap_or_default();
    ThreadLifecycleLog {
        event: "thread.reuse",
        session_id,
        developer_instructions,
    }
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

pub(crate) fn interrogate_expectation_with_model<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    expectation: &SelectedExpectation,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    history_cache: &mut HistoryCache,
    enforced_scope: &[String],
    model: Option<&str>,
) -> Result<InterrogationResult, EvaluatorError> {
    // Expectation mode may start from a history-derived restricted scope, but
    // after sanitization this path shares query mode's first-turn construction:
    // developer instructions and the turn prompt are rendered from
    // `resources/prompts/` plus runtime data.
    let enforced_scope = sanitize_scope(enforced_scope)?;
    let against_tree_answer = against_tree_answer_with_cache(
        runtime.root,
        &runtime.tree_context.against_tree,
        &expectation.agent,
        expectation,
        &enforced_scope,
        history_cache,
        &mut state.visible_tree_oid_cache,
    )
    .map_err(EvaluatorError::message)?;
    let prompt = evaluator_turn_prompt(&expectation.q, against_tree_answer.as_ref())
        .map_err(EvaluatorError::message)?;
    let thinking = effective_thinking(&expectation.agent, expectation);
    let response = ask_with_reused_thread(
        runtime,
        runner,
        diagnostic_log,
        state,
        ThreadTurnRequest {
            agent: &expectation.agent,
            enforced_scope: &enforced_scope,
            model,
            thinking,
            expectation_id: Some(&expectation.id),
            prompt: &prompt,
        },
    )?;
    finalize_interrogation_response(
        runtime,
        expectation,
        diagnostic_log,
        state,
        &enforced_scope,
        response,
    )
}
