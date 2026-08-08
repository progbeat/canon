mod dynamic_tool;
mod failure;
mod text;
mod usage;

pub(crate) use dynamic_tool::{dynamic_tool_call_response, take_dynamic_tool_call};
pub(crate) use failure::{app_server_error_value, app_server_failure_from_value};
pub(crate) use text::turn_text;
pub(crate) use usage::{
    context_compaction_event, token_usage_update, turn_started_id,
    UnsequencedContextCompactionEvent, UnsequencedTokenUsageUpdate,
};

use serde_json::Value;

// Each incoming Value is one protocol event. This component performs every
// deterministic interpretation once for that event, and transport and usage
// consumers reuse the resulting AppServerMessage. It does not inspect
// repositories or filesystems or derive hashes.
#[derive(Debug)]
pub(crate) struct AppServerMessage<'a> {
    pub(crate) raw: &'a Value,
    pub(crate) request_id: Option<u64>,
    pub(crate) response_id: Option<u64>,
    pub(crate) method: Option<&'a str>,
    pub(crate) kind: AppServerEventKind,
    pub(crate) params: Option<&'a Value>,
    pub(crate) result: Option<&'a Value>,
    pub(crate) error: Option<&'a Value>,
    dynamic_tool_call: Option<Result<dynamic_tool::ParsedDynamicToolCall, String>>,
    // Normalize text payloads as part of the single message parse so every
    // consumer reuses the parsed view instead of inspecting the JSON again.
    pub(crate) agent_message_delta_text: Option<&'a str>,
    pub(crate) agent_message_completed_text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppServerEventKind {
    AgentMessageCompleted,
    AgentMessageDelta,
    ContextCompaction,
    DynamicToolCall,
    Error,
    ItemCompleted,
    TokenUsageUpdated,
    TurnCompleted,
    TurnError,
    TurnFailed,
    TurnStarted,
    Unclassified,
}

impl AppServerEventKind {
    fn from_method(method: Option<&str>) -> Self {
        match method {
            Some("item/agentMessage/completed") => Self::AgentMessageCompleted,
            Some("item/agentMessage/delta") => Self::AgentMessageDelta,
            Some(
                "thread/contextCompaction/created"
                | "thread/contextCompaction/updated"
                | "thread/contextCompaction/completed"
                | "turn/contextCompaction/created"
                | "turn/contextCompaction/completed"
                | "contextCompaction/created"
                | "contextCompaction/completed",
            ) => Self::ContextCompaction,
            Some("item/tool/call") => Self::DynamicToolCall,
            Some("error") => Self::Error,
            Some("item/completed") => Self::ItemCompleted,
            Some("thread/tokenUsage/updated") => Self::TokenUsageUpdated,
            Some("turn/completed") => Self::TurnCompleted,
            Some("turn/error") => Self::TurnError,
            Some("turn/failed") => Self::TurnFailed,
            Some("turn/started") => Self::TurnStarted,
            _ => Self::Unclassified,
        }
    }
}

pub(crate) fn app_server_message(value: &Value) -> Result<AppServerMessage<'_>, String> {
    value.as_object().ok_or_else(|| {
        "failed to parse app-server message envelope: expected object".to_string()
    })?;
    let method = optional_str_field(value, "method")?;
    let kind = AppServerEventKind::from_method(method);
    let params = optional_value_field(value, "params");
    let agent_text = text::parse_agent_text(kind, params);
    let dynamic_tool_call = dynamic_tool::parse_dynamic_tool_call(kind, params);
    let id = optional_u64_field(value, "id")?;
    let (request_id, response_id) = if method.is_some() {
        (id, None)
    } else {
        (None, id)
    };
    let message = AppServerMessage {
        raw: value,
        request_id,
        response_id,
        method,
        kind,
        params,
        result: optional_value_field(value, "result"),
        error: optional_value_field(value, "error"),
        dynamic_tool_call,
        agent_message_delta_text: agent_text.agent_message_delta_text,
        agent_message_completed_text: agent_text.agent_message_completed_text,
    };
    if id.is_none() && message.method.is_none() {
        return Err("app-server message envelope missing both id and method".to_string());
    }
    Ok(message)
}

fn optional_u64_field(value: &Value, key: &str) -> Result<Option<u64>, String> {
    let Some(field) = optional_value_field(value, key) else {
        return Ok(None);
    };
    field.as_u64().map(Some).ok_or_else(|| {
        format!("failed to parse app-server message envelope: `{key}` must be an unsigned integer")
    })
}

fn optional_str_field<'a>(value: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    let Some(field) = optional_value_field(value, key) else {
        return Ok(None);
    };
    field.as_str().map(Some).ok_or_else(|| {
        format!("failed to parse app-server message envelope: `{key}` must be a string")
    })
}

fn optional_value_field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|field| !field.is_null())
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn string_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    value_at_path(value, path).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test] // xpec: gN
    fn request_and_response_ids_keep_distinct_protocol_meanings() {
        let response_value = json!({"id": 7, "result": {}});
        let request_value = json!({
            "id": 7,
            "method": "item/tool/call",
            "params": {}
        });
        let response = app_server_message(&response_value).unwrap();
        let request = app_server_message(&request_value).unwrap();

        assert_eq!(response.response_id, Some(7));
        assert_eq!(response.request_id, None);
        assert_eq!(request.response_id, None);
        assert_eq!(request.request_id, Some(7));
    }
}
