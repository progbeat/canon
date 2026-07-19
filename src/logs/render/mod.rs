mod escape;
mod event;
mod schema;
mod token_usage;

use crate::logs::error::{external_log_error, DiagnosticLogResult};
use crate::time::{format_record_timestamp, unix_timestamp};
use serde_json::Value;

pub(crate) use escape::push_json_control_escape;

pub(crate) struct RenderedRuntimeLogEvent {
    pub(crate) line: String,
}

#[cfg(test)]
pub(crate) fn render_runtime_log_event(
    level: &str,
    event: &str,
    fields: &[(&str, Value)],
) -> DiagnosticLogResult<String> {
    Ok(render_runtime_log_event_with_process(None, None, level, event, fields)?.line)
}

pub(crate) fn render_runtime_log_process_event(
    invocation_id: &str,
    level: &str,
    event: &str,
    fields: &[(&str, Value)],
) -> DiagnosticLogResult<RenderedRuntimeLogEvent> {
    // [7N,g2,m8,R1] Native process and invocation IDs are primary correlation
    // fields for concurrent processes and sequential command runs. They link
    // events without deriving a second representation from event timestamps.
    render_runtime_log_event_with_process(
        Some(std::process::id()),
        Some(invocation_id),
        level,
        event,
        fields,
    )
}

fn render_runtime_log_event_with_process(
    process_id: Option<u32>,
    invocation_id: Option<&str>,
    level: &str,
    event: &str,
    fields: &[(&str, Value)],
) -> DiagnosticLogResult<RenderedRuntimeLogEvent> {
    // Runtime Logs requires `level` and `event` to be single-line labels.
    schema::validate_runtime_log_common_fields(level, event)?;
    schema::validate_runtime_log_extra_fields(fields)?;
    schema::validate_runtime_log_event_schema(event, fields)?;
    let timestamp = format_record_timestamp(
        unix_timestamp().map_err(|message| external_log_error("read system time", message))?,
    );
    let event = event::RuntimeLogEvent {
        timestamp: timestamp.clone(),
        level,
        event,
        process_id,
        invocation_id,
        extra: fields,
    };
    Ok(RenderedRuntimeLogEvent {
        line: event::json_line(&event, "runtime log event")?,
    })
}

#[cfg(test)]
mod tests {
    use super::render_runtime_log_event;
    use serde_json::{json, Value};

    #[test] // xpec: hJ,7N,m8
    fn agent_token_usage_accepts_aggregate_with_turn_context() {
        let fields = vec![
            ("id", json!("id")),
            ("attempt", json!(1)),
            ("reason", json!("initial")),
            ("threadId", json!("thread")),
            ("turnId", json!("turn")),
            ("tokenUsage", token_usage()),
        ];

        render_runtime_log_event("info", "agent.token_usage", &fields).unwrap();
    }

    #[test] // xpec: m8
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

    #[test] // xpec: m8
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

    #[test] // xpec: m8
    fn check_finish_does_not_require_derived_status() {
        let fields = vec![("query", json!(false))];

        render_runtime_log_event("info", "check.finish", &fields).unwrap();
    }

    #[test] // xpec: m8
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

    #[test] // xpec: m8
    fn thread_lifecycle_events_include_reuse_context() {
        let fields = vec![
            ("threadId", json!("thread")),
            ("id", json!("id")),
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
