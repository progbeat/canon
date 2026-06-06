use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct TokenUsage {
    pub(crate) total_tokens: u64,
    pub(crate) input_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_output_tokens: u64,
}

const UNCACHED_INPUT_1M_REFERENCE_PRICE: f64 = 1.0;
const CACHED_INPUT_1M_REFERENCE_PRICE: f64 = 0.1;
const OUTPUT_1M_REFERENCE_PRICE: f64 = 10.0;

pub(crate) fn reference_token_cost(
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
) -> f64 {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TokenUsageUpdate {
    pub(crate) sequence: u64,
    #[serde(rename = "threadId")]
    pub(crate) thread_id: String,
    #[serde(rename = "turnId")]
    pub(crate) turn_id: String,
    #[serde(rename = "tokenUsage")]
    pub(crate) token_usage: Value,
    #[serde(skip)]
    pub(crate) last_usage: TokenUsage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ContextCompactionEvent {
    pub(crate) sequence: u64,
    #[serde(rename = "threadId")]
    pub(crate) thread_id: String,
    #[serde(rename = "turnId")]
    pub(crate) turn_id: String,
    pub(crate) method: String,
    pub(crate) event: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvaluatorTurnUsage {
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) usage: TokenUsage,
    pub(crate) token_usage_updates: Vec<TokenUsageUpdate>,
    pub(crate) context_compaction_events: Vec<ContextCompactionEvent>,
}
