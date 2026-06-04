mod cache_key;
mod cleanup;
mod reuse;
mod store;

pub(crate) use cache_key::history_cache_key;
pub(crate) use cleanup::{active_expectation_ids_from_identities, cleanup_stale_cache_dirs};
pub(crate) use reuse::{
    cached_history_record, cooldown_history_record, is_reusable_history_record,
    latest_history_record_matching_visible_tree_oid, latest_stored_q_scope_with_cache,
    same_tree_history_record_with_cache, CachedHistoryRecord,
};
pub(crate) use store::{append_current_history_record_with_cache, HistoryCache};
