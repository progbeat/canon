use crate::check_interrogation_records::finalize_interrogation_response;
use crate::check_interrogation_state::{evaluator_session_key, CheckRuntime, InterrogationState};
use crate::check_model_fallback::write_model_fallback_events;
use crate::check_types::{InterrogationResult, SelectedExpectation};
use crate::config_types::AgentConfig;
use crate::evaluator_prompt::{developer_instructions, EVALUATOR_BASE_INSTRUCTIONS};
use crate::evaluator_response_cache::EvaluatorResponseParseCache;
use crate::evaluator_turn::{
    ask_once, effective_thinking, is_context_window_failure, model_label,
    session_failure_invalidates_thread, EvaluatorTurnContext, ParsedTurnResponse,
};
use crate::evaluator_types::{EvaluatorError, EvaluatorRunner};
use crate::logging::DiagnosticLogWriter;
use crate::scope::sanitize_scope;
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
pub(crate) struct ThreadTurnRequest<'a> {
    pub(crate) enforced_scope: &'a [String],
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
    pub(crate) expectation_id: Option<&'a str>,
    pub(crate) prompt: &'a str,
}

struct ThreadLifecycleLog {
    event: &'static str,
    session_id: String,
    developer_instructions: String,
}

pub(crate) fn ask_with_reused_thread<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
    request: ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let config = runtime.config;
    let session_key = evaluator_session_key(request.enforced_scope);
    // Threads are reused only to preserve the same enforced scope and rendered
    // developer instructions. Each turn still sends the current expectation
    // prompt as the only active task input.
    let existing_session = state.sessions_by_scope.get(&session_key).cloned();
    let had_existing_session = existing_session.is_some();
    let lifecycle_log = match existing_session {
        Some(existing) => thread_reuse_log(state, &config.agent, existing, request),
        None => start_thread_session(runtime, runner, state, &session_key, request)?,
    };
    let mut session_id = lifecycle_log.session_id.clone();
    write_thread_lifecycle_event(diagnostic_log, &lifecycle_log, request)?;
    let response = match ask_current_session(
        runner,
        &session_id,
        &config.agent,
        state,
        diagnostic_log,
        request,
    ) {
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
            write_thread_restart_event(diagnostic_log, &session_id, request, err.message_str())?;
            let lifecycle_log =
                start_thread_session(runtime, runner, state, &session_key, request)?;
            session_id = lifecycle_log.session_id.clone();
            write_thread_lifecycle_event(diagnostic_log, &lifecycle_log, request)?;
            match ask_current_session(
                runner,
                &session_id,
                &config.agent,
                state,
                diagnostic_log,
                request,
            ) {
                Ok(response) => response,
                Err(err) => return fail_after_session_error(state, err),
            }
        }
        Err(err) => return fail_after_session_error(state, err),
    };
    if !retire_thread_sessions_after_turn(state, runner.take_retired_sessions(), &session_id) {
        state.sessions_by_scope.insert(session_key, session_id);
    }
    Ok(response)
}

fn ask_current_session<R: EvaluatorRunner>(
    runner: &mut R,
    session_id: &str,
    agent: &AgentConfig,
    state: &mut InterrogationState,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    request: ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    ask_in_thread(
        runner,
        session_id,
        agent,
        &mut state.parse_cache,
        diagnostic_log,
        request,
    )
}

fn ask_in_thread<R: EvaluatorRunner>(
    runner: &mut R,
    session_id: &str,
    agent: &AgentConfig,
    parse_cache: &mut EvaluatorResponseParseCache,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    request: ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let turn = EvaluatorTurnContext {
        session_id,
        model: request.model,
        thinking: request.thinking,
    };
    ask_once(
        runner,
        &turn,
        request.prompt,
        agent,
        parse_cache,
        diagnostic_log,
        request.expectation_id,
    )
}

fn start_thread_session<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    state: &mut InterrogationState,
    session_key: &str,
    request: ThreadTurnRequest<'_>,
) -> Result<ThreadLifecycleLog, EvaluatorError> {
    let config = runtime.config;
    let developer_instructions = developer_instructions(&config.agent, request.enforced_scope);
    let created = match runner.start_session(
        runtime.snapshot_root,
        &developer_instructions,
        &config.agent,
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
        .sessions_by_scope
        .insert(session_key.to_string(), created.clone());
    Ok(ThreadLifecycleLog {
        event: "thread.start",
        session_id: created,
        developer_instructions,
    })
}

fn thread_reuse_log(
    state: &InterrogationState,
    agent: &AgentConfig,
    session_id: String,
    request: ThreadTurnRequest<'_>,
) -> ThreadLifecycleLog {
    let developer_instructions = state
        .session_instructions
        .get(&session_id)
        .cloned()
        .unwrap_or_else(|| developer_instructions(agent, request.enforced_scope));
    ThreadLifecycleLog {
        event: "thread.reuse",
        session_id,
        developer_instructions,
    }
}

fn write_thread_lifecycle_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    lifecycle_log: &ThreadLifecycleLog,
    request: ThreadTurnRequest<'_>,
) -> Result<(), String> {
    write_thread_event(
        diagnostic_log,
        "info",
        lifecycle_log.event,
        &[
            ("threadId", json!(&lifecycle_log.session_id)),
            ("scope", json!(request.enforced_scope)),
            ("model", json!(model_label(request.model))),
            ("thinking", json!(request.thinking)),
            ("baseInstructions", json!(EVALUATOR_BASE_INSTRUCTIONS)),
            (
                "developerInstructions",
                json!(&lifecycle_log.developer_instructions),
            ),
        ],
    )
}

fn write_thread_restart_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    session_id: &str,
    request: ThreadTurnRequest<'_>,
    reason: &str,
) -> Result<(), String> {
    write_thread_event(
        diagnostic_log,
        "warn",
        "thread.restart",
        &[
            ("threadId", json!(session_id)),
            ("id", json!(request.expectation_id)),
            ("scope", json!(request.enforced_scope)),
            ("model", json!(model_label(request.model))),
            ("reason", json!(reason)),
        ],
    )
}

fn write_thread_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    level: &str,
    event: &str,
    fields: &[(&str, Value)],
) -> Result<(), String> {
    let Some(writer) = diagnostic_log.as_deref_mut() else {
        return Ok(());
    };
    writer
        .write_event(level, event, fields)
        .map_err(|err| err.to_string())
}

fn clear_thread_sessions_after_failure(state: &mut InterrogationState) {
    // Same-scope reuse applies to successful, still-live evaluator threads.
    // Technical app-server failures can retire the backing process, so keeping
    // the old session ID would point at a stale or missing thread rather than
    // preserving the same Codex thread.
    state.clear_thread_sessions();
}

fn retire_thread_sessions_after_turn(
    state: &mut InterrogationState,
    retired_sessions: Vec<String>,
    active_session_id: &str,
) -> bool {
    if retired_sessions.is_empty() {
        return false;
    }
    let retired_sessions = retired_sessions.into_iter().collect::<BTreeSet<_>>();
    state
        .sessions_by_scope
        .retain(|_, session_id| !retired_sessions.contains(session_id));
    state
        .session_instructions
        .retain(|session_id, _| !retired_sessions.contains(session_id));
    retired_sessions.contains(active_session_id)
}

fn fail_after_session_error<T>(
    state: &mut InterrogationState,
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
    state: &mut InterrogationState,
    enforced_scope: &[String],
    model: Option<&str>,
) -> Result<InterrogationResult, EvaluatorError> {
    let config = runtime.config;
    // Expectation mode may start from a history-derived restricted scope, but
    // after sanitization this path shares query mode's first-turn construction:
    // developer instructions are determined by agent config plus enforced
    // scope, and the task prompt is exactly the expectation question.
    let enforced_scope = sanitize_scope(enforced_scope, &config.agent)?;
    let prompt = expectation.q.clone();
    let thinking = effective_thinking(&config.agent, expectation);
    let response = ask_with_reused_thread(
        runtime,
        runner,
        diagnostic_log,
        state,
        ThreadTurnRequest {
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
