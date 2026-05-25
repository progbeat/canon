use crate::check_types::{CheckRecord, SelectedExpectation};
use crate::config_types::AgentConfig;
use crate::history::HistoryCache;
use crate::history_reuse::{cooldown_history_record, same_tree_history_record_with_cache};
use crate::logging::DiagnosticLogWriter;
use crate::time::parse_record_timestamp;
use crate::visible_tree_oid::VisibleTreeOidCache;
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
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    lookup: CachedResultLookup,
) -> Result<Option<CheckCacheHit>, String> {
    let same_tree = if lookup.include_same_tree {
        same_tree_history_record_with_cache(
            root,
            agent,
            expectation,
            history_cache,
            visible_tree_oid_cache,
        )?
        .map(|record| CheckCacheHit {
            record,
            kind: CachedResultKind::SameTree,
        })
    } else {
        None
    };
    let cooldown = if lookup.include_cooldown {
        cooldown_history_record(root, agent, expectation, history_cache, lookup.now)?.map(
            |record| CheckCacheHit {
                record,
                kind: CachedResultKind::Cooldown,
            },
        )
    } else {
        None
    };
    Ok(newer_cache_hit(same_tree, cooldown))
}

fn newer_cache_hit(
    same_tree: Option<CheckCacheHit>,
    cooldown: Option<CheckCacheHit>,
) -> Option<CheckCacheHit> {
    match (same_tree, cooldown) {
        (Some(same_tree), Some(cooldown)) => {
            if record_timestamp_sort_key(&cooldown.record)
                > record_timestamp_sort_key(&same_tree.record)
            {
                Some(cooldown)
            } else {
                Some(same_tree)
            }
        }
        (Some(hit), None) | (None, Some(hit)) => Some(hit),
        (None, None) => None,
    }
}

fn record_timestamp_sort_key(record: &CheckRecord) -> u64 {
    parse_record_timestamp(&record.timestamp).unwrap_or(0)
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
