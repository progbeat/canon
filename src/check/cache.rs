use crate::check::types::{CheckRecord, SelectedExpectation};
use crate::config_types::AgentConfig;
use crate::git::tree_source::TreeSource;
use crate::git::visible_tree_oid::VisibleTreeOidCache;
use crate::history::reuse::{
    cooldown_history_record, newer_cached_history_record, same_tree_history_record_with_cache,
    CachedHistoryRecord,
};
use crate::history::HistoryCache;
use crate::logs::DiagnosticLogWriter;
use serde_json::json;
use std::path::Path;

pub(crate) struct CheckCacheHit {
    pub(crate) record: CheckRecord,
    pub(crate) kind: CachedResultKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedResultKind {
    SameTree,
    Cooldown,
}

pub(crate) struct CachedResultLookup {
    pub(crate) now: u64,
    pub(crate) include_same_tree: bool,
    pub(crate) include_cooldown: bool,
}

pub(crate) fn cached_result_for_expectation(
    root: &Path,
    source: &TreeSource,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    lookup: CachedResultLookup,
) -> Result<Option<CheckCacheHit>, String> {
    let same_tree = if lookup.include_same_tree {
        same_tree_history_record_with_cache(
            root,
            source,
            agent,
            expectation,
            history_cache,
            visible_tree_oid_cache,
        )?
    } else {
        None
    };
    let cooldown = if lookup.include_cooldown {
        cooldown_history_record(root, agent, expectation, history_cache, lookup.now)?
    } else {
        None
    };
    Ok(
        newer_cached_history_record(same_tree, cooldown).map(|hit| match hit {
            CachedHistoryRecord::SameTree(record) => CheckCacheHit {
                record,
                kind: CachedResultKind::SameTree,
            },
            CachedHistoryRecord::Cooldown(record) => CheckCacheHit {
                record,
                kind: CachedResultKind::Cooldown,
            },
        }),
    )
}

pub(crate) fn write_cache_hit(
    writer: &mut DiagnosticLogWriter,
    hit: &CheckCacheHit,
) -> Result<(), String> {
    writer
        .write_event(
            "info",
            "cache.hit",
            &[
                ("id", json!(hit.record.id)),
                ("result", json!(hit.record.result)),
                ("scope", json!(hit.record.scope)),
                ("kind", json!(format!("{:?}", hit.kind))),
            ],
        )
        .map_err(|err| err.to_string())?;
    writer
        .write_record(&hit.record)
        .map_err(|err| err.to_string())
}
