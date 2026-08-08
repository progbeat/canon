use crate::app::protocol::{dynamic_tool_call_response, take_dynamic_tool_call, AppServerMessage};
use crate::evaluator::{EvaluatorDynamicToolHandler, EvaluatorDynamicToolResult};
use serde_json::Value;

pub(super) fn handle_dynamic_tool_call(
    message: &mut AppServerMessage<'_>,
    handler: &mut Option<&mut dyn EvaluatorDynamicToolHandler>,
) -> Value {
    let result = match take_dynamic_tool_call(message) {
        Ok(call) => match handler {
            Some(handler) => handler.handle_dynamic_tool_call(call),
            None => EvaluatorDynamicToolResult::failure(
                "dynamic tool calls are not available for this evaluator turn",
            ),
        },
        Err(err) => EvaluatorDynamicToolResult::failure(err),
    };
    dynamic_tool_call_response(result)
}
