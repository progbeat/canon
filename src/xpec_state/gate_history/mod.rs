//! Bounded per-xpec Git-backed history used by the fast gate.
//!
//! Canonical Last Results remain the latest results across every check mode.
//! This cache preserves the gate-relevant fields from the latest Git-backed
//! result of each status when an in-place check subsequently updates those
//! canonical files.
//! Gate-history entries are not Last Results or Cached Results: matching one
//! supports only gate's regression comparison and never constitutes reuse of a
//! canonical same-tree result.

mod cache;
mod migration;
mod model;
mod persistence;
#[cfg(test)]
mod tests;

pub(super) use migration::preserve_canonical_results;
pub(crate) use model::GateHistory;
pub(super) use persistence::CACHE_FILE_NAME;
