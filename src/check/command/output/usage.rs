use crate::token_usage_types::TokenUsage;

pub(crate) fn render_token_usage_summary(usage: TokenUsage) -> String {
    format!(
        "Token usage: total={} input={} (+ {} cached) output={} (reasoning {})",
        usage.total_tokens,
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.output_tokens,
        usage.reasoning_output_tokens
    )
}
