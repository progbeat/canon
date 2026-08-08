use super::{AppServerEventKind, AppServerMessage};
use crate::evaluator::{EvaluatorDynamicToolCall, EvaluatorDynamicToolResult};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
pub(super) struct ParsedDynamicToolCall {
    #[serde(rename = "threadId")]
    _thread_id: String,
    #[serde(rename = "turnId")]
    _turn_id: String,
    #[serde(rename = "callId")]
    _call_id: String,
    namespace: Option<String>,
    tool: String,
    #[serde(default)]
    arguments: Value,
}

pub(super) fn parse_dynamic_tool_call(
    kind: AppServerEventKind,
    params: Option<&Value>,
) -> Option<Result<ParsedDynamicToolCall, String>> {
    if kind != AppServerEventKind::DynamicToolCall {
        return None;
    }
    Some(
        params
            .ok_or_else(|| "dynamic tool call is missing params".to_string())
            .and_then(|params| {
                // The parsed value is stored on AppServerMessage and moved to
                // the handler; this is the payload's only typed conversion.
                serde_json::from_value::<ParsedDynamicToolCall>(params.clone())
                    .map_err(|err| format!("failed to parse dynamic tool call: {}", err))
            }),
    )
}

pub(crate) fn take_dynamic_tool_call(
    message: &mut AppServerMessage<'_>,
) -> Result<EvaluatorDynamicToolCall, String> {
    if message.kind != AppServerEventKind::DynamicToolCall {
        return Err("app-server message is not a dynamic tool call".to_string());
    }
    let params = message
        .dynamic_tool_call
        .take()
        .ok_or_else(|| "app-server dynamic tool call payload was already consumed".to_string())??;
    Ok(EvaluatorDynamicToolCall {
        namespace: params.namespace,
        tool: params.tool,
        arguments: params.arguments,
    })
}

pub(crate) fn dynamic_tool_call_response(result: EvaluatorDynamicToolResult) -> Value {
    json!({
        "contentItems": [
            {
                "type": "inputText",
                "text": result.text
            }
        ],
        "success": result.success
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: 6
    fn dynamic_tool_call_parses_namespaced_tool_request() {
        let message = json!({
            "method": "item/tool/call",
            "params": {
                "threadId": "thread",
                "turnId": "turn",
                "callId": "call",
                "namespace": "canon",
                "tool": "show",
                "arguments": {
                    "selectors": ["abc"]
                }
            }
        });
        let mut message = crate::app::protocol::app_server_message(&message).unwrap();
        let call = take_dynamic_tool_call(&mut message).unwrap();

        assert_eq!(call.namespace.as_deref(), Some("canon"));
        assert_eq!(call.tool, "show");
        assert_eq!(call.arguments["selectors"], json!(["abc"]));
    }

    #[test] // xpec: 6
    fn dynamic_tool_call_response_uses_app_server_content_items() {
        let response =
            dynamic_tool_call_response(EvaluatorDynamicToolResult::success("show output"));

        assert_eq!(
            response,
            json!({
                "contentItems": [
                    {
                        "type": "inputText",
                        "text": "show output"
                    }
                ],
                "success": true
            })
        );
    }
}
