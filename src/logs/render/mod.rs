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
    // Runtime Logs requires `level` and `event` to be single-line labels.
    schema::validate_runtime_log_common_fields(level, event)?;
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
    use crate::token_usage_types::{TokenUsage, TokenUsageUpdate};
    use serde_json::{json, Value};

    #[test]
    fn agent_response_accepts_typed_token_usage_updates() {
        let fields = agent_response_fields(json!(vec![TokenUsageUpdate {
            sequence: 1,
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            token_usage: json!({
                "last": token_usage(),
                "total": token_usage(),
            }),
            last_usage: TokenUsage::default(),
        }]));

        render_runtime_log_event("info", "agent.response", &fields).unwrap();
    }

    #[test]
    fn agent_response_allows_missing_usage_when_unavailable() {
        let fields = vec![
            ("id", json!("id")),
            ("attempt", json!(1)),
            ("reason", json!("initial")),
            ("error", json!("missing evaluator turn usage")),
            ("response", json!({"sessionId": "thread", "text": "{}"})),
        ];

        render_runtime_log_event("error", "agent.turn_error", &fields).unwrap();
    }

    #[test]
    fn review_required_record_events_include_reason() {
        let fields = vec![
            ("id", json!("id")),
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
    // xpec: B
    fn runtime_log_common_fields_are_single_line_labels() {
        let level_error = render_runtime_log_event("warn\nnext", "check.start", &[]).unwrap_err();
        let event_error = render_runtime_log_event("warn", "check.start\rnext", &[]).unwrap_err();
        let tab_error = render_runtime_log_event("warn\t", "check.start", &[]).unwrap_err();
        let empty_error = render_runtime_log_event("warn", "", &[]).unwrap_err();
        let separator_error =
            render_runtime_log_event("warn", "check.start\u{2028}next", &[]).unwrap_err();

        assert_eq!(
            level_error.to_string(),
            "runtime log field \"level\" is not a single-line label"
        );
        assert_eq!(
            event_error.to_string(),
            "runtime log field \"event\" is not a single-line label"
        );
        assert_eq!(
            tab_error.to_string(),
            "runtime log field \"level\" is not a single-line label"
        );
        assert_eq!(
            empty_error.to_string(),
            "runtime log field \"event\" is not a single-line label"
        );
        assert_eq!(
            separator_error.to_string(),
            "runtime log field \"event\" is not a single-line label"
        );
    }

    #[test]
    fn thread_lifecycle_events_include_reuse_context() {
        let fields = vec![
            ("threadId", json!("thread")),
            ("scope", json!(["."])),
            ("model", json!("model")),
            ("thinking", json!("medium")),
            ("baseInstructions", json!("base")),
            ("developerInstructions", json!("developer")),
            (
                "reuseContext",
                json!({
                    "visibleTreeOid": "visible",
                    "diffBaseTreeOid": "base-tree",
                    "checkedTreeOid": "checked-tree",
                    "turnPrompt": "prompt",
                    "questionContext": "context",
                    "plugins": ["plugin"],
                    "ignore": ["target/**"],
                }),
            ),
        ];

        render_runtime_log_event("info", "thread.start", &fields).unwrap();
        render_runtime_log_event("info", "thread.reuse", &fields).unwrap();
    }

    fn agent_response_fields(updates: Value) -> Vec<(&'static str, Value)> {
        vec![
            ("id", json!("id")),
            ("attempt", json!(1)),
            ("reason", json!("initial")),
            ("response", json!({"sessionId": "thread", "text": "{}"})),
            ("tokenUsageUpdates", updates),
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
