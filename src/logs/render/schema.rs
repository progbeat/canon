use super::token_usage::validate_agent_token_usage_schema;
use crate::logs::error::{DiagnosticLogError, DiagnosticLogResult};
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) fn validate_runtime_log_event_schema(
    event: &str,
    fields: &[(&str, Value)],
) -> DiagnosticLogResult<()> {
    let Some(required) = required_runtime_log_fields(event) else {
        return Ok(());
    };
    for key in required {
        if runtime_log_field_value(fields, key).is_some() {
            continue;
        }
        return Err(DiagnosticLogError::InvalidRuntimeField {
            key: (*key).to_string(),
            reason: "missing for event schema",
        });
    }
    validate_agent_token_usage_schema(event, fields)?;
    Ok(())
}

fn required_runtime_log_fields(event: &str) -> Option<&'static [&'static str]> {
    // This table is the schema map for known runtime-log events. Thread
    // lifecycle events require the effective base/developer instructions for
    // inspection, and `validate_agent_token_usage_schema` below validates
    // per-turn token usage fields for agent response/error events.
    match event {
        "agent.request" => Some(&["id", "attempt", "reason", "request"]),
        "agent.response" => Some(&["id", "attempt", "reason", "response"]),
        "agent.turn_error" => Some(&["id", "attempt", "reason", "response"]),
        "cache.cleanup" => Some(&["removed", "kept"]),
        "cache.hit" => Some(&["id", "result", "scope"]),
        "check.start" => Some(&["selected"]),
        "expectation.result" | "interrogation.result" => Some(&[
            "id", "result", "observed", "evidence", "scope", "prompt", "expected",
        ]),
        "expectation.review_required" | "interrogation.review_required" => Some(&[
            "id", "result", "observed", "evidence", "scope", "prompt", "expected", "reason",
        ]),
        "model.failure" => Some(&["id", "model", "error"]),
        "model.fallback" => Some(&["id", "from", "to", "reason"]),
        "query.result" => Some(&["prompt", "observed", "evidence", "qScopeSuggestion"]),
        "query.review_required" => Some(&[
            "prompt",
            "observed",
            "evidence",
            "qScopeSuggestion",
            "reason",
        ]),
        "scope.narrowing" => Some(&["id", "originalScope", "proposedScope", "accepted"]),
        "thread.restart" => Some(&[
            "threadId",
            "id",
            "scope",
            "model",
            "baseInstructions",
            "developerInstructions",
            "reason",
        ]),
        "thread.start" | "thread.reuse" => Some(&[
            "threadId",
            "scope",
            "model",
            "thinking",
            "baseInstructions",
            "developerInstructions",
        ]),
        "check.finish" => Some(&["query"]),
        _ => None,
    }
}

pub(super) fn runtime_log_field_value<'a>(
    fields: &'a [(&str, Value)],
    key: &str,
) -> Option<&'a Value> {
    fields
        .iter()
        .find_map(|(field, value)| (*field == key).then_some(value))
}

pub(super) fn validate_runtime_log_extra_fields(
    fields: &[(&str, Value)],
) -> DiagnosticLogResult<()> {
    let mut seen = BTreeSet::new();
    for (key, _) in fields {
        if matches!(*key, "timestamp" | "level" | "event") {
            return Err(DiagnosticLogError::InvalidRuntimeField {
                key: (*key).to_string(),
                reason: "reserved",
            });
        }
        if !seen.insert(*key) {
            return Err(DiagnosticLogError::InvalidRuntimeField {
                key: (*key).to_string(),
                reason: "duplicated",
            });
        }
    }
    Ok(())
}
