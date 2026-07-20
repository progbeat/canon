use crate::token_usage_types::TokenUsage;

pub(crate) fn render_token_usage_summary(usage: TokenUsage) -> String {
    format!(
        "token-usage: ref-cost={:.2}$ total={} input={} (+ {} cached) output={} (reasoning {})",
        usage.reference_token_cost(),
        usage.total_tokens,
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.output_tokens,
        usage.reasoning_output_tokens
    )
}

#[cfg(test)]
mod tests {
    use super::render_token_usage_summary;
    use crate::token_usage_types::TokenUsage;

    #[test] // xpec: 9b,8J
    fn token_usage_output_matches_documented_line() {
        let usage = TokenUsage {
            total_tokens: 9,
            input_tokens: 4,
            cached_input_tokens: 3,
            output_tokens: 2,
            reasoning_output_tokens: 1,
        };

        assert_eq!(
            render_token_usage_summary(usage),
            "token-usage: ref-cost=0.00$ total=9 input=4 (+ 3 cached) output=2 (reasoning 1)"
        );
    }
}
