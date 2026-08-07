use super::TokenUsage;

const UNCACHED_INPUT_1M_REFERENCE_PRICE: f64 = 1.0;
const CACHED_INPUT_1M_REFERENCE_PRICE: f64 = 0.1;
const OUTPUT_1M_REFERENCE_PRICE: f64 = 10.0;
const TOKENS_PER_MILLION: f64 = 1_000_000.0;

impl TokenUsage {
    pub(crate) fn reference_token_cost(self) -> f64 {
        // xpec: Uh
        assert!(
            self.cached_input_tokens <= self.input_tokens,
            "cached_input_tokens cannot exceed input_tokens"
        );
        let uncached_input_tokens = self.input_tokens - self.cached_input_tokens;

        (uncached_input_tokens as f64 * UNCACHED_INPUT_1M_REFERENCE_PRICE
            + self.cached_input_tokens as f64 * CACHED_INPUT_1M_REFERENCE_PRICE
            + self.output_tokens as f64 * OUTPUT_1M_REFERENCE_PRICE)
            / TOKENS_PER_MILLION
    }
}

#[cfg(test)]
mod tests {
    use super::TokenUsage;

    #[test] // xpec: Uh
    fn reference_cost_uses_canonical_prices() {
        let usage = TokenUsage {
            input_tokens: 2_000_000,
            cached_input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            ..TokenUsage::default()
        };

        assert_eq!(usage.reference_token_cost(), 11.1);
    }

    #[test] // xpec: Uh
    #[should_panic(expected = "cached_input_tokens cannot exceed input_tokens")]
    fn reference_cost_rejects_cached_input_above_input() {
        TokenUsage {
            input_tokens: 1,
            cached_input_tokens: 2,
            ..TokenUsage::default()
        }
        .reference_token_cost();
    }
}
