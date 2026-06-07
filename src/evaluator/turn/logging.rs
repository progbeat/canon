use super::{EvaluatorFailureKind, EvaluatorTurnContext, RawTurnResponse, ThreadLifecycleLog};
use crate::evaluator::prompt::EVALUATOR_BASE_INSTRUCTIONS;
use crate::evaluator::types::{EvaluatorError, EvaluatorRunner};
use crate::logs::DiagnosticLogWriter;
use crate::token_usage_types::{EvaluatorTurnUsage, TokenUsage};
use serde::Serialize;
use serde_json::{json, Value};

pub(super) fn ask_and_log<R: EvaluatorRunner>(
    runner: &mut R,
    turn: &EvaluatorTurnContext<'_>,
    prompt: &str,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
) -> Result<RawTurnResponse, EvaluatorError> {
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        let raw_request = serde_json::to_value(EvaluatorTurnLogRequest {
            session_id: turn.session_id,
            prompt,
            model: turn.model,
            thinking: turn.thinking,
        })
        .map_err(|err| format!("failed to encode evaluator turn request log: {}", err))?;
        writer.write_event(
            "info",
            "agent.request",
            &[
                ("id", json!(expectation_id)),
                ("attempt", json!(attempt)),
                ("reason", json!(reason)),
                ("request", raw_request),
            ],
        )?;
    }
    let response = match runner.ask(turn.session_id, prompt, turn.model, turn.thinking) {
        Ok(response) => response,
        Err(err) => {
            let turn_usage = runner.take_last_turn_usage();
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                let raw_response = json!({
                    "sessionId": turn.session_id,
                    "error": err.message_str(),
                });
                let mut fields = vec![
                    ("id", json!(expectation_id)),
                    ("attempt", json!(attempt)),
                    ("reason", json!(reason)),
                    ("error", json!(err.message_str())),
                    ("response", raw_response),
                ];
                append_turn_usage_fields(&mut fields, turn_usage.as_ref());
                let event = if turn_usage.is_some() {
                    "agent.response"
                } else {
                    append_missing_turn_usage_fields(&mut fields, turn.session_id);
                    "agent.turn_error"
                };
                writer.write_event("error", event, &fields)?;
            }
            return Err(err);
        }
    };
    let turn_usage = runner.take_last_turn_usage();
    let response_usage = turn_usage.as_ref().map(|turn_usage| turn_usage.usage);
    let missing_turn_usage = diagnostic_log.is_some() && turn_usage.is_none();
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        let raw_response = json!({
            "sessionId": turn.session_id,
            "text": response.clone(),
        });
        let mut fields: Vec<(&'static str, Value)> = vec![
            ("id", json!(expectation_id)),
            ("attempt", json!(attempt)),
            ("reason", json!(reason)),
            ("response", raw_response),
        ];
        append_turn_usage_fields(&mut fields, turn_usage.as_ref());
        if missing_turn_usage {
            // A response without usage violates the app-server turn contract,
            // so it is not logged as a completed `agent.response`.
            fields.push(("error", json!("missing evaluator turn usage")));
            append_missing_turn_usage_fields(&mut fields, turn.session_id);
            writer.write_event("error", "agent.turn_error", &fields)?;
        } else {
            writer.write_event("info", "agent.response", &fields)?;
        }
    }
    if missing_turn_usage {
        return Err(EvaluatorError::failure(
            EvaluatorFailureKind::UnknownAppServer,
            "missing evaluator turn usage",
        ));
    }
    let context_compacted = turn_usage
        .as_ref()
        .is_some_and(|turn_usage| !turn_usage.context_compaction_events.is_empty());
    Ok(RawTurnResponse {
        text: response,
        usage: response_usage,
        context_compacted,
    })
}

pub(crate) fn write_thread_lifecycle_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    lifecycle_log: &ThreadLifecycleLog,
    enforced_scope: &[String],
    model: Option<&str>,
    thinking: &str,
) -> Result<(), String> {
    write_thread_event(
        diagnostic_log,
        "info",
        lifecycle_log.event,
        &[
            ("threadId", json!(&lifecycle_log.session_id)),
            ("scope", json!(enforced_scope)),
            ("model", json!(model)),
            ("thinking", json!(thinking)),
            ("baseInstructions", json!(EVALUATOR_BASE_INSTRUCTIONS)),
            (
                "developerInstructions",
                json!(&lifecycle_log.developer_instructions),
            ),
        ],
    )
}

pub(crate) fn write_thread_restart_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    session_id: &str,
    expectation_id: Option<&str>,
    enforced_scope: &[String],
    model: Option<&str>,
    developer_instructions: &str,
    reason: &str,
) -> Result<(), String> {
    write_thread_event(
        diagnostic_log,
        "warn",
        "thread.restart",
        &[
            ("threadId", json!(session_id)),
            ("id", json!(expectation_id)),
            ("scope", json!(enforced_scope)),
            ("model", json!(model)),
            ("baseInstructions", json!(EVALUATOR_BASE_INSTRUCTIONS)),
            ("developerInstructions", json!(developer_instructions)),
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
    fields.push(("tokenUsageUnavailable", json!(true)));
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

#[derive(Serialize)]
struct EvaluatorTurnLogRequest<'a> {
    #[serde(rename = "sessionId")]
    session_id: &'a str,
    prompt: &'a str,
    model: Option<&'a str>,
    thinking: &'a str,
}
