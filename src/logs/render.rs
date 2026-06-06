mod escape;
mod event;
mod schema;
mod token_usage;

use crate::logs::error::{external_log_error, DiagnosticLogResult};
use crate::time::{format_record_timestamp, unix_timestamp};
use serde_json::Value;

pub(crate) use escape::push_json_control_escape;

pub(crate) fn render_runtime_log_event(
    level: &str,
    event: &str,
    fields: &[(&str, Value)],
) -> DiagnosticLogResult<String> {
    schema::validate_runtime_log_extra_fields(fields)?;
    schema::validate_runtime_log_event_schema(event, fields)?;
    let event = event::RuntimeLogEvent {
        timestamp: format_record_timestamp(
            unix_timestamp().map_err(|message| external_log_error("read system time", message))?,
        ),
        level,
        event,
        extra: fields,
    };
    event::json_line(&event, "runtime log event")
}

#[cfg(test)]
mod tests {
    use super::render_runtime_log_event;
    use serde_json::{json, Value};

    #[test]
    fn token_usage_updates_expose_raw_count_objects() {
        let fields = agent_response_fields(vec![json!({
            "sequence": 1,
            "threadId": "thread",
            "turnId": "turn",
            "tokenUsage": {
                "last": token_usage(),
                "total": token_usage(),
            },
        })]);

        render_runtime_log_event("info", "agent.response", &fields).unwrap();
    }

    #[test]
    fn token_usage_updates_require_reasoning_count() {
        let mut usage = token_usage();
        usage
            .as_object_mut()
            .unwrap()
            .remove("reasoningOutputTokens");
        let fields = agent_response_fields(vec![json!({
            "sequence": 1,
            "threadId": "thread",
            "turnId": "turn",
            "tokenUsage": {
                "last": usage,
                "total": token_usage(),
            },
        })]);

        let error = render_runtime_log_event("info", "agent.response", &fields).unwrap_err();

        assert!(error.to_string().contains("reasoningOutputTokens"));
    }

    #[test]
    fn agent_response_allows_missing_usage_when_unavailable() {
        let fields = vec![
            ("id", json!("id")),
            ("attempt", json!(1)),
            ("reason", json!("initial")),
            ("error", json!("missing evaluator turn usage")),
            ("response", json!({"sessionId": "thread", "text": "{}"})),
            ("tokenUsageUnavailable", json!(true)),
        ];

        render_runtime_log_event("error", "agent.turn_error", &fields).unwrap();
    }

    #[test]
    fn review_required_record_events_include_reason() {
        let fields = vec![
            ("id", json!("id")),
            ("result", json!("fail")),
            ("observed", json!("unparsable")),
            ("evidence", json!("evidence")),
            ("scope", json!(["."])),
            ("prompt", json!("prompt")),
            ("expected", json!("yes")),
            ("reason", json!("unparsable")),
        ];

        render_runtime_log_event("warn", "expectation.review_required", &fields).unwrap();
    }

    #[test]
    fn check_finish_does_not_require_derived_status() {
        let fields = vec![("query", json!(false))];

        render_runtime_log_event("info", "check.finish", &fields).unwrap();
    }

    #[test]
    fn thread_events_accept_raw_default_model_as_null() {
        let fields = vec![
            ("threadId", json!("thread")),
            ("scope", json!(["."])),
            ("model", Value::Null),
            ("thinking", json!("medium")),
            ("baseInstructions", json!("base")),
            ("developerInstructions", json!("developer")),
        ];

        render_runtime_log_event("info", "thread.start", &fields).unwrap();
    }

    fn agent_response_fields(updates: Vec<Value>) -> Vec<(&'static str, Value)> {
        vec![
            ("id", json!("id")),
            ("attempt", json!(1)),
            ("reason", json!("initial")),
            ("response", json!({"sessionId": "thread", "text": "{}"})),
            ("tokenUsageUpdates", json!(updates)),
        ]
    }

    fn token_usage() -> Value {
        json!({
            "totalTokens": 10,
            "inputTokens": 6,
            "cachedInputTokens": 2,
            "outputTokens": 4,
            "reasoningOutputTokens": 1,
        })
    }
}
