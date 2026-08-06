//! Fixed reference pricing for token usage.

use super::TokenUsage;

const UNCACHED_INPUT_1M_REFERENCE_PRICE: f64 = 1.0;
const CACHED_INPUT_1M_REFERENCE_PRICE: f64 = 0.1;
const OUTPUT_1M_REFERENCE_PRICE: f64 = 10.0;

fn reference_token_cost(input_tokens: u64, cached_input_tokens: u64, output_tokens: u64) -> f64 {
    // xpec: Uh
    assert!(
        cached_input_tokens <= input_tokens,
        "cached_input_tokens cannot exceed input_tokens"
    );
    let uncached_input = input_tokens - cached_input_tokens;
    (uncached_input as f64 * UNCACHED_INPUT_1M_REFERENCE_PRICE
        + cached_input_tokens as f64 * CACHED_INPUT_1M_REFERENCE_PRICE
        + output_tokens as f64 * OUTPUT_1M_REFERENCE_PRICE)
        / 1_000_000.0
}

impl TokenUsage {
    pub(crate) fn reference_token_cost(self) -> f64 {
        // Canon stores uncached input and cached input separately, matching the
        // public `input=<n> (+ <n> cached)` summary. The reference-cost spec's
        // `input_tokens` parameter is total input including cached tokens.
        reference_token_cost(
            self.input_tokens + self.cached_input_tokens,
            self.cached_input_tokens,
            self.output_tokens,
        )
    }
}
