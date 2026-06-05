use crate::app::APP_SERVER_TURN_TIMEOUT_SECS;
use crate::evaluator::{EvaluatorError, EvaluatorFailureKind};
use crate::token_usage_types::{ContextCompactionEvent, TokenUsage, TokenUsageUpdate};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub(crate) fn turn_idle_timed_out(last_activity: Instant, now: Instant) -> bool {
    now.duration_since(last_activity) >= Duration::from_secs(APP_SERVER_TURN_TIMEOUT_SECS)
}

#[derive(Debug, Deserialize)]
pub(crate) struct AppServerMessage {
    pub(crate) id: Option<u64>,
    pub(crate) method: Option<String>,
    #[serde(default)]
    pub(crate) result: Option<Value>,
    #[serde(default)]
    pub(crate) error: Option<Value>,
}

pub(crate) fn app_server_message(value: &Value) -> Result<AppServerMessage, String> {
    let message = serde_json::from_value::<AppServerMessage>(value.clone())
        .map_err(|err| format!("failed to parse app-server message envelope: {}", err))?;
    if message.id.is_none() && message.method.is_none() {
        return Err("app-server message envelope missing both id and method".to_string());
    }
    Ok(message)
}

#[derive(Deserialize)]
struct AgentMessageDeltaMessage {
    method: Option<String>,
    params: Option<AgentMessageDeltaParams>,
}

#[derive(Deserialize)]
struct AgentMessageDeltaParams {
    delta: Option<String>,
}

pub(crate) fn agent_message_delta(value: &Value) -> Option<String> {
    let message = serde_json::from_value::<AgentMessageDeltaMessage>(value.clone()).ok()?;
    if message.method.as_deref() != Some("item/agentMessage/delta") {
        return None;
    }
    message.params?.delta
}

pub(crate) fn app_server_failure_kind(error: &Value) -> EvaluatorFailureKind {
    let code = serde_json::from_value::<AppServerErrorFields>(error.clone())
        .ok()
        .and_then(AppServerErrorFields::code);
    code.as_deref()
        .map(app_server_failure_kind_from_code)
        .unwrap_or(EvaluatorFailureKind::UnknownAppServer)
}

pub(crate) fn app_server_failure_from_value(method: &str, error: &Value) -> EvaluatorError {
    let failure = format!("app-server {} failed: {}", method, error);
    EvaluatorError::failure(app_server_failure_kind(error), failure)
}

#[cfg(test)]
pub(crate) fn app_server_failure_from_message(method: &str, message: &str) -> EvaluatorError {
    let failure = format!("app-server {} failed: {}", method, message);
    EvaluatorError::message(failure)
}

pub(crate) fn app_server_failure_kind_from_code(code: &str) -> EvaluatorFailureKind {
    match code {
        "usageLimitExceeded" | "usage_limit_exceeded" => EvaluatorFailureKind::UsageLimit,
        "rateLimitExceeded" | "rate_limit_exceeded" => EvaluatorFailureKind::RateLimit,
        "modelUnavailable" | "model_unavailable" => EvaluatorFailureKind::ModelUnavailable,
        "contextWindowExceeded" | "context_window_exceeded" | "context_length_exceeded" => {
            EvaluatorFailureKind::ContextWindow
        }
        _ => EvaluatorFailureKind::UnknownAppServer,
    }
}

#[derive(Deserialize)]
struct AppServerErrorFields {
    code: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "codexErrorInfo")]
    codex_error_info: Option<String>,
}

impl AppServerErrorFields {
    fn code(self) -> Option<String> {
        self.code.or(self.kind).or(self.codex_error_info)
    }
}

impl TokenUsage {
    pub(crate) fn add(self, other: TokenUsage) -> TokenUsage {
        TokenUsage {
            total_tokens: self.total_tokens + other.total_tokens,
            input_tokens: self.input_tokens + other.input_tokens,
            cached_input_tokens: self.cached_input_tokens + other.cached_input_tokens,
            output_tokens: self.output_tokens + other.output_tokens,
            reasoning_output_tokens: self.reasoning_output_tokens + other.reasoning_output_tokens,
        }
    }
}

pub(crate) fn token_usage_update(message: &Value) -> Option<TokenUsageUpdate> {
    if message.get("method").and_then(Value::as_str) != Some("thread/tokenUsage/updated") {
        return None;
    }
    let params = message.get("params")?;
    let thread_id = params.get("threadId").and_then(Value::as_str)?.to_string();
    let turn_id = params.get("turnId").and_then(Value::as_str)?.to_string();
    let token_usage = params.get("tokenUsage")?.clone();
    let last_usage = parse_token_usage(token_usage.get("last")?)?;
    parse_token_usage(token_usage.get("total")?)?;
    Some(TokenUsageUpdate {
        sequence: 0,
        thread_id,
        turn_id,
        token_usage,
        last_usage,
    })
}

pub(crate) fn context_compaction_event(message: &Value) -> Option<ContextCompactionEvent> {
    let method = message.get("method").and_then(Value::as_str)?;
    let params = message.get("params")?;
    let is_compaction_item = params
        .get("item")
        .and_then(|item| item.get("type"))
        .and_then(Value::as_str)
        .is_some_and(is_compaction_item_type);
    if !is_compaction_item && !is_context_compaction_method(method) {
        return None;
    }
    let thread_id = string_at_path(params, &["threadId"])
        .or_else(|| string_at_path(params, &["thread", "id"]))?
        .to_string();
    let turn_id = string_at_path(params, &["turnId"])
        .or_else(|| string_at_path(params, &["turn", "id"]))?
        .to_string();
    Some(ContextCompactionEvent {
        sequence: 0,
        thread_id,
        turn_id,
        method: method.to_string(),
        event: message.clone(),
    })
}

fn is_compaction_item_type(kind: &str) -> bool {
    matches!(kind, "contextCompaction" | "compacted")
}

fn is_context_compaction_method(method: &str) -> bool {
    matches!(
        method,
        "thread/contextCompaction/created"
            | "thread/contextCompaction/updated"
            | "thread/contextCompaction/completed"
            | "turn/contextCompaction/created"
            | "turn/contextCompaction/completed"
            | "contextCompaction/created"
            | "contextCompaction/completed"
    )
}

pub(crate) fn turn_started_id(message: &Value) -> Option<String> {
    if message.get("method").and_then(Value::as_str) != Some("turn/started") {
        return None;
    }
    message
        .get("params")?
        .get("turn")?
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub(crate) fn parse_token_usage(value: &Value) -> Option<TokenUsage> {
    // This only parses app-server usage payloads. The public `canon check`
    // output contract is implemented in `check_output`:
    // per-expectation stdout records, the token-usage stderr line, and the
    // final summary line. This protocol module intentionally has no
    // `render_token_usage_summary` or `format_number`; `check_output` renders
    // token counts as raw decimal integers with no thousands separators.
    // `check_command` owns the command-level write order.
    Some(TokenUsage {
        total_tokens: value.get("totalTokens").and_then(Value::as_u64)?,
        input_tokens: value.get("inputTokens").and_then(Value::as_u64)?,
        cached_input_tokens: value
            .get("cachedInputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: value.get("outputTokens").and_then(Value::as_u64)?,
        reasoning_output_tokens: value
            .get("reasoningOutputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

#[cfg(test)]
pub(crate) fn app_server_error_message(message: &Value) -> Option<String> {
    app_server_error_value(message).map(|error| app_server_error_display(&error))
}

pub(crate) fn app_server_error_value(message: &Value) -> Option<Value> {
    let method = message.get("method").and_then(Value::as_str)?;
    if method != "error" && method != "turn/failed" && method != "turn/error" {
        if method == "turn/completed"
            && string_at_path(message, &["params", "turn", "status"]) == Some("failed")
        {
            return message
                .get("params")?
                .get("turn")?
                .get("error")
                .cloned()
                .or_else(|| Some(json!({ "message": "turn failed" })));
        }
        return None;
    }
    value_at_path(message, &["params", "error"])
        .or_else(|| value_at_path(message, &["error"]))
        .cloned()
        .or_else(|| string_at_path(message, &["params", "message"]).map(message_error_value))
        .or_else(|| string_at_path(message, &["message"]).map(message_error_value))
        .or_else(|| Some(message_error_value(method)))
}

#[cfg(test)]
pub(crate) fn app_server_error_display(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string())
}

pub(crate) fn message_error_value(message: &str) -> Value {
    json!({ "message": message })
}

pub(crate) fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

pub(crate) fn string_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    value_at_path(value, path).and_then(Value::as_str)
}

pub(crate) fn turn_text(delta_text: String, completed_text: String) -> String {
    if delta_text.trim().is_empty() {
        completed_text
    } else {
        delta_text
    }
}

pub(crate) fn append_completed_agent_text(message: &Value, output: &mut String) {
    let Some(params) = message.get("params") else {
        return;
    };
    if let Some(item) = params.get("item") {
        if is_assistant_message_item(item) {
            append_message_payload_text(item, output);
        }
    } else if message.get("method").and_then(Value::as_str) == Some("item/agentMessage/completed") {
        append_message_payload_text(params, output);
    }
}

pub(crate) fn is_assistant_message_item(item: &Value) -> bool {
    item.get("role").and_then(Value::as_str) == Some("assistant")
        || item
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(is_assistant_message_type)
}

fn is_assistant_message_type(kind: &str) -> bool {
    matches!(
        kind,
        "agentMessage" | "agent_message" | "assistantMessage" | "assistant_message"
    )
}

pub(crate) fn append_message_payload_text(payload: &Value, output: &mut String) {
    if let Some(text) = payload.get("text").and_then(Value::as_str) {
        output.push_str(text);
    }
    if let Some(content) = payload.get("content").and_then(Value::as_array) {
        append_content_text_parts(content, output);
    }
}

pub(crate) fn append_content_text_parts(parts: &[Value], output: &mut String) {
    for part in parts {
        let Some(kind) = part.get("type").and_then(Value::as_str) else {
            continue;
        };
        if matches!(kind, "output_text" | "text") {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                output.push_str(text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_compaction_event_requires_exact_method_or_item_type() {
        let unrelated = json!({
            "method": "thread/compactDisc/updated",
            "params": {
                "threadId": "thread",
                "turnId": "turn"
            }
        });
        assert!(context_compaction_event(&unrelated).is_none());

        let method_event = json!({
            "method": "thread/contextCompaction/updated",
            "params": {
                "threadId": "thread",
                "turnId": "turn"
            }
        });
        assert!(context_compaction_event(&method_event).is_some());

        let item_event = json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread",
                "turnId": "turn",
                "item": { "type": "contextCompaction" }
            }
        });
        assert!(context_compaction_event(&item_event).is_some());
    }

    #[test]
    fn assistant_message_item_type_uses_exact_names() {
        assert!(is_assistant_message_item(&json!({"type": "agentMessage"})));
        assert!(is_assistant_message_item(&json!({"role": "assistant"})));
        assert!(!is_assistant_message_item(
            &json!({"type": "agent_status_message"})
        ));
    }
}
