use super::{string_at_path, AppServerEventKind, AppServerMessage};
use crate::token_usage::TokenUsage;
use serde_json::Value;

pub(crate) struct UnsequencedTokenUsageUpdate {
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) token_usage: Value,
    pub(crate) thread_total_usage: TokenUsage,
}

pub(crate) struct UnsequencedContextCompactionEvent {
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) method: String,
    pub(crate) event: Value,
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

pub(crate) fn token_usage_update(
    message: &AppServerMessage<'_>,
) -> Option<UnsequencedTokenUsageUpdate> {
    if message.kind != AppServerEventKind::TokenUsageUpdated {
        return None;
    }
    let params = message.params?;
    let thread_id = params.get("threadId").and_then(Value::as_str)?.to_string();
    let turn_id = params.get("turnId").and_then(Value::as_str)?.to_string();
    let token_usage = params.get("tokenUsage")?.clone();
    parse_token_usage(token_usage.get("last")?)?;
    let thread_total_usage = parse_token_usage(token_usage.get("total")?)?;
    Some(UnsequencedTokenUsageUpdate {
        thread_id,
        turn_id,
        token_usage,
        thread_total_usage,
    })
}

pub(crate) fn context_compaction_event(
    message: &AppServerMessage<'_>,
) -> Option<UnsequencedContextCompactionEvent> {
    let method = message.method?;
    let params = message.params?;
    let is_compaction_item = params
        .get("item")
        .and_then(|item| item.get("type"))
        .and_then(Value::as_str)
        .is_some_and(is_compaction_item_type);
    if !is_compaction_item && message.kind != AppServerEventKind::ContextCompaction {
        return None;
    }
    let thread_id = string_at_path(params, &["threadId"])
        .or_else(|| string_at_path(params, &["thread", "id"]))?
        .to_string();
    let turn_id = string_at_path(params, &["turnId"])
        .or_else(|| string_at_path(params, &["turn", "id"]))?
        .to_string();
    Some(UnsequencedContextCompactionEvent {
        thread_id,
        turn_id,
        method: method.to_string(),
        event: message.raw.clone(),
    })
}

fn is_compaction_item_type(kind: &str) -> bool {
    matches!(kind, "contextCompaction" | "compacted")
}

pub(crate) fn turn_started_id(message: &AppServerMessage<'_>) -> Option<String> {
    if message.kind != AppServerEventKind::TurnStarted {
        return None;
    }
    message
        .params?
        .get("turn")?
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub(crate) fn parse_token_usage(value: &Value) -> Option<TokenUsage> {
    // This only parses app-server usage payloads. The public `canon check`
    // output contract is implemented in `check::command::output`:
    // per-expectation stdout records, the token-usage stderr line, and the
    // final summary line. This protocol module intentionally has no
    // `render_token_usage_summary` or `format_number`;
    // `check::command::output` renders token counts as raw decimal integers
    // with no thousands separators. `check::command::{workflow, completion}`
    // own the command-level write order.
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test] // xpec: qv
    fn context_compaction_event_requires_exact_method_or_item_type() {
        let unrelated = json!({
            "method": "thread/compactDisc/updated",
            "params": {
                "threadId": "thread",
                "turnId": "turn"
            }
        });
        let unrelated = crate::app::protocol::app_server_message(&unrelated).unwrap();
        assert!(context_compaction_event(&unrelated).is_none());

        let method_event = json!({
            "method": "thread/contextCompaction/updated",
            "params": {
                "threadId": "thread",
                "turnId": "turn"
            }
        });
        let method_event = crate::app::protocol::app_server_message(&method_event).unwrap();
        assert!(context_compaction_event(&method_event).is_some());

        let item_event = json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread",
                "turnId": "turn",
                "item": { "type": "contextCompaction" }
            }
        });
        let item_event = crate::app::protocol::app_server_message(&item_event).unwrap();
        assert!(context_compaction_event(&item_event).is_some());
    }
}
