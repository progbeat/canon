//! Token usage values and reference pricing.

mod reference_cost;

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct TokenUsage {
    pub(crate) total_tokens: u64,
    pub(crate) input_tokens: u64,
    // Cached input is the cached subset of `input_tokens`, not an additional count.
    pub(crate) cached_input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_output_tokens: u64,
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
    // Parsed from the cumulative app-server `tokenUsage.total` snapshot. The
    // process usage ledger converts successive snapshots into per-turn usage.
    #[serde(skip)]
    pub(crate) thread_total_usage: TokenUsage,
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
