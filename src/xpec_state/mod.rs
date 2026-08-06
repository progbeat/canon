//! Persistent state for collected xpecs.
//!
//! The modules own the cache view, cached-pass decisions, last-result files,
//! bounded Git-backed gate data, failure history, and configuration-wide
//! retention. Every normal stateful check runs retention before evaluation and
//! rejects later writes unless they belong to that successfully retained
//! complete configuration.

mod cache;
mod cached_pass;
mod fail_history;
mod gate_history;
mod last_result;
mod retention;

pub(crate) use cache::XpecStateCache;
pub(crate) use cached_pass::{
    cached_pass_result_for_expectation, check_record_from_cached_pass_result,
    latest_fail_timestamp, refresh_reused_same_tree_pass_result,
    response_timestamp_is_within_cooldown, stored_visible_scope_matches_checked_tree,
    CachedPassResultKind, CachedPassResultLookup,
};
pub(crate) use fail_history::FailureHistoryFeedback;
pub(crate) use gate_history::GateHistory;
#[cfg(test)]
pub(crate) use last_result::LastResultResponse;
pub(crate) use last_result::{LastResult, LastResultStatus};
pub(crate) use retention::{
    prune_uncollected_in_place_xpec_state, prune_uncollected_xpec_state_dirs,
};
