use crate::logs::error::{DiagnosticLogError, DiagnosticLogResult};
use crate::logs::DiagnosticLogWriter;
use crate::token_usage_types::{EvaluatorTurnUsage, TokenUsage};
use serde::Serialize;
use serde_json::{json, Value};

// Check lifecycle event helpers are routed through
// `check::interrogation::session` so lifecycle logging is visible beside
// evaluator communication logging while the JSON event schemas stay centralized.
pub(crate) fn write_check_start_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: Option<bool>,
    selected: Vec<String>,
) -> DiagnosticLogResult<()> {
    let mut fields = Vec::new();
    if let Some(query) = query {
        fields.push(("query", json!(query)));
    }
    fields.push(("selected", json!(selected)));
    diagnostic_log.write_event("info", "check.start", &fields)
}

pub(crate) fn write_query_start_event(
    diagnostic_log: &mut DiagnosticLogWriter,
) -> DiagnosticLogResult<()> {
    write_check_start_event(diagnostic_log, Some(true), Vec::new())
}

pub(crate) fn write_query_finish_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    err: Option<&str>,
) -> DiagnosticLogResult<()> {
    write_check_finish_event(diagnostic_log, true, err)
}

pub(crate) fn write_check_finish_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    error: Option<&str>,
) -> DiagnosticLogResult<()> {
    let mut fields = vec![("query", json!(query))];
    if let Some(error) = error {
        fields.push(("error", json!(error)));
    }
    diagnostic_log.write_event("info", "check.finish", &fields)
}

pub(crate) fn write_cache_cleanup_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    removed: usize,
    kept: usize,
) -> DiagnosticLogResult<()> {
    diagnostic_log.write_event(
        "info",
        "cache.cleanup",
        &[("removed", json!(removed)), ("kept", json!(kept))],
    )
}

pub(crate) fn write_agent_request_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    request: AgentTurnLogRequest<'_>,
) -> DiagnosticLogResult<()> {
    let raw_request = serde_json::to_value(request).map_err(|source| DiagnosticLogError::Json {
        description: "evaluator turn request log",
        source,
    })?;
    diagnostic_log.write_event(
        "info",
        "agent.request",
        &[
            ("id", json!(expectation_id)),
            ("attempt", json!(attempt)),
            ("reason", json!(reason)),
            ("request", raw_request),
        ],
    )
}

pub(crate) fn write_agent_failure_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    session_id: &str,
    error: &str,
    turn_usage: Option<&EvaluatorTurnUsage>,
) -> DiagnosticLogResult<()> {
    let raw_response = json!({
        "sessionId": session_id,
        "error": error,
    });
    let mut fields = vec![
        ("id", json!(expectation_id)),
        ("attempt", json!(attempt)),
        ("reason", json!(reason)),
        ("response", raw_response),
    ];
    append_turn_usage_fields(&mut fields, turn_usage);
    let event = if turn_usage.is_some() {
        "agent.response"
    } else {
        append_missing_turn_usage_fields(&mut fields, session_id);
        "agent.turn_error"
    };
    diagnostic_log.write_event("error", event, &fields)
}

pub(crate) fn write_agent_response_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    session_id: &str,
    response: &str,
    turn_usage: &EvaluatorTurnUsage,
) -> DiagnosticLogResult<()> {
    let raw_response = json!({
        "sessionId": session_id,
        "text": response,
    });
    let mut fields: Vec<(&'static str, Value)> = vec![
        ("id", json!(expectation_id)),
        ("attempt", json!(attempt)),
        ("reason", json!(reason)),
        ("response", raw_response),
    ];
    append_turn_usage_fields(&mut fields, Some(turn_usage));
    diagnostic_log.write_event("info", "agent.response", &fields)
}

pub(crate) fn write_agent_missing_usage_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    session_id: &str,
    response: &str,
) -> DiagnosticLogResult<()> {
    let raw_response = json!({
        "sessionId": session_id,
        "text": response,
    });
    let mut fields: Vec<(&'static str, Value)> = vec![
        ("id", json!(expectation_id)),
        ("attempt", json!(attempt)),
        ("reason", json!(reason)),
        ("response", raw_response),
        ("error", json!("missing evaluator turn usage")),
    ];
    append_missing_turn_usage_fields(&mut fields, session_id);
    diagnostic_log.write_event("error", "agent.turn_error", &fields)
}

pub(crate) fn write_thread_lifecycle_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    event: &ThreadLifecycleEventFields<'_>,
) -> DiagnosticLogResult<()> {
    diagnostic_log.write_event(
        "info",
        event.event,
        &[
            ("threadId", json!(event.session_id)),
            ("scope", json!(event.scope)),
            ("model", json!(event.model)),
            ("thinking", json!(event.thinking)),
            ("baseInstructions", json!(event.base_instructions)),
            ("developerInstructions", json!(event.developer_instructions)),
            ("reuseContext", json!(event.reuse_context)),
        ],
    )
}

pub(crate) fn write_thread_restart_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    event: &ThreadRestartEventFields<'_>,
) -> DiagnosticLogResult<()> {
    diagnostic_log.write_event(
        "warn",
        "thread.restart",
        &[
            ("threadId", json!(event.session_id)),
            ("id", json!(event.expectation_id)),
            ("scope", json!(event.scope)),
            ("model", json!(event.model)),
            ("baseInstructions", json!(event.base_instructions)),
            ("developerInstructions", json!(event.developer_instructions)),
            ("reason", json!(event.reason)),
        ],
    )
}

#[derive(Serialize)]
pub(crate) struct AgentTurnLogRequest<'a> {
    #[serde(rename = "sessionId")]
    pub(crate) session_id: &'a str,
    pub(crate) prompt: &'a str,
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
}

pub(crate) struct ThreadLifecycleEventFields<'a> {
    pub(crate) event: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) scope: &'a [String],
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
    pub(crate) base_instructions: &'a str,
    pub(crate) developer_instructions: &'a str,
    pub(crate) reuse_context: &'a crate::evaluator::ThreadReuseLogContext,
}

pub(crate) struct ThreadRestartEventFields<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) expectation_id: Option<&'a str>,
    pub(crate) scope: &'a [String],
    pub(crate) model: Option<&'a str>,
    pub(crate) base_instructions: &'a str,
    pub(crate) developer_instructions: &'a str,
    pub(crate) reason: &'a str,
}

fn append_turn_usage_fields(
    fields: &mut Vec<(&'static str, Value)>,
    turn_usage: Option<&EvaluatorTurnUsage>,
) {
    let Some(EvaluatorTurnUsage {
        thread_id,
        turn_id,
        usage,
        token_usage_updates,
        context_compaction_events,
        ..
    }) = turn_usage
    else {
        return;
    };
    fields.push(("threadId", json!(thread_id)));
    fields.push(("turnId", json!(turn_id)));
    if !token_usage_updates.is_empty() {
        fields.push(("tokenUsageUpdates", json!(token_usage_updates)));
    } else {
        fields.push(("tokenUsage", token_usage_log_value(*usage)));
    }
    if !context_compaction_events.is_empty() {
        fields.push(("contextCompactionEvents", json!(context_compaction_events)));
    }
}

fn append_missing_turn_usage_fields(fields: &mut Vec<(&'static str, Value)>, session_id: &str) {
    fields.push(("threadId", json!(session_id)));
}

fn token_usage_log_value(usage: TokenUsage) -> Value {
    json!({
        "totalTokens": usage.total_tokens,
        "inputTokens": usage.input_tokens,
        "cachedInputTokens": usage.cached_input_tokens,
        "outputTokens": usage.output_tokens,
        "reasoningOutputTokens": usage.reasoning_output_tokens,
    })
}
