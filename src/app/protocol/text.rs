use super::AppServerEventKind;
use serde_json::Value;

pub(super) struct ParsedAgentText<'a> {
    pub(super) agent_message_delta_text: Option<&'a str>,
    pub(super) agent_message_completed_text: Option<String>,
}

pub(super) fn parse_agent_text<'a>(
    kind: AppServerEventKind,
    params: Option<&'a Value>,
) -> ParsedAgentText<'a> {
    let agent_message_delta_text = if kind == AppServerEventKind::AgentMessageDelta {
        params
            .and_then(|params| params.get("delta"))
            .and_then(Value::as_str)
    } else {
        None
    };
    let agent_message_completed_text = if matches!(
        kind,
        AppServerEventKind::ItemCompleted | AppServerEventKind::AgentMessageCompleted
    ) {
        agent_message_completed_text(kind, params)
    } else {
        None
    };
    ParsedAgentText {
        agent_message_delta_text,
        agent_message_completed_text,
    }
}

pub(crate) fn turn_text(
    agent_message_delta_text: String,
    agent_message_completed_text: String,
) -> String {
    if agent_message_completed_text.trim().is_empty() {
        agent_message_delta_text
    } else {
        agent_message_completed_text
    }
}

fn agent_message_completed_text(
    kind: AppServerEventKind,
    params: Option<&Value>,
) -> Option<String> {
    let params = params?;
    let payload = if let Some(item) = params.get("item") {
        if is_assistant_message_item(item) {
            Some(item)
        } else {
            None
        }
    } else if kind == AppServerEventKind::AgentMessageCompleted {
        Some(params)
    } else {
        None
    };
    let payload = payload?;
    let mut text = String::new();
    append_message_payload_text(payload, &mut text);
    (!text.trim().is_empty()).then_some(text)
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
    use serde_json::json;

    #[test] // xpec: qv
    fn assistant_message_item_type_uses_exact_names() {
        assert!(is_assistant_message_item(&json!({"type": "agentMessage"})));
        assert!(is_assistant_message_item(&json!({"role": "assistant"})));
        assert!(!is_assistant_message_item(
            &json!({"type": "agent_status_message"})
        ));
    }

    #[test] // xpec: qv
    fn latest_completed_agent_message_supplies_agent_message_completed_text() {
        let status_message = json!({
            "method": "item/completed",
            "params": {
                "item": {
                    "type": "agentMessage",
                    "content": [{"type": "output_text", "text": "status"}]
                }
            }
        });
        let answer_message = json!({
            "method": "item/completed",
            "params": {
                "item": {
                    "type": "agentMessage",
                    "content": [{"type": "output_text", "text": "{\"answer\":\"no\"}"}]
                }
            }
        });
        let status_message = crate::app::protocol::app_server_message(&status_message).unwrap();
        let answer_message = crate::app::protocol::app_server_message(&answer_message).unwrap();

        let mut agent_message_completed_text = String::new();
        for message in [status_message, answer_message] {
            if let Some(message_text) = message.agent_message_completed_text {
                agent_message_completed_text = message_text;
            }
        }

        assert_eq!(agent_message_completed_text, "{\"answer\":\"no\"}");
    }

    #[test] // xpec: qv
    fn turn_text_prefers_completed_message_over_delta_stream() {
        assert_eq!(
            turn_text(
                "status{\"answer\":\"yes\"}".to_string(),
                "{\"answer\":\"yes\"}".to_string()
            ),
            "{\"answer\":\"yes\"}"
        );
    }
}
