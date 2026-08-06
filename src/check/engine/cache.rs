//! Cached results for the canon's expectation-and-Git-state domain.
//!
//! An in-place invocation has no Git state and therefore never performs
//! this component's cached-result lookup; its config contract also rejects
//! `cooldown`. That command boundary does not add a provenance condition to a
//! later Git-backed lookup: the canonical cooldown rule uses the latest pass
//! and its response timestamp.

use crate::check::core::{CachedPassRecord, ResolvedExpectation};
use crate::check::interrogation::write_expectation_result_event;
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::logs::DiagnosticLogWriter;
use crate::xpec_state::{
    cached_pass_result_for_expectation, check_record_from_cached_pass_result,
    refresh_reused_same_tree_pass_result, CachedPassResultKind, CachedPassResultLookup,
    XpecStateCache,
};
use serde_json::json;
use std::path::Path;

// Every operation in this module requires a Git `TreeSource`; in-place
// execution cannot call this API.
pub(crate) struct CheckCacheHit {
    pub(crate) pass_record: CachedPassRecord,
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
    checked_tree_oid: &str,
    expectation: &ResolvedExpectation,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    lookup: CachedResultLookup,
) -> Result<Option<CheckCacheHit>, String> {
    if !lookup.include_same_tree && !lookup.include_cooldown {
        return Ok(None);
    }
    // A cache hit is converted straight into the CheckRecord that the check
    // output layer prints. It does not render evaluator prompts; prompt-only
    // inputs such as `diff-from` apply only to fresh evaluator turns.
    let Some(hit) = cached_pass_result_for_expectation(
        root,
        source,
        expectation,
        xpec_state,
        visible_tree_oid_cache,
        CachedPassResultLookup {
            now: lookup.now,
            include_same_tree: lookup.include_same_tree,
            include_cooldown: lookup.include_cooldown,
        },
    )?
    else {
        return Ok(None);
    };
    let hit =
        refresh_reused_same_tree_pass_result(root, checked_tree_oid, expectation, xpec_state, hit)?;
    let record =
        check_record_from_cached_pass_result(root, expectation, &hit, visible_tree_oid_cache)?;
    // [m] Fail or human-review records are history, not Cached Results. Treat
    // any legacy candidate shaped that way as uncached so a fresh evaluator
    // run can produce the current-run output.
    let Some(pass_record) = CachedPassRecord::from_cache_candidate(record) else {
        return Ok(None);
    };
    let kind = match hit.kind {
        CachedPassResultKind::SameTree => CachedResultKind::SameTree,
        CachedPassResultKind::Cooldown => CachedResultKind::Cooldown,
    };
    Ok(Some(CheckCacheHit { pass_record, kind }))
}

pub(crate) fn write_cache_hit(
    writer: &mut DiagnosticLogWriter,
    hit: &CheckCacheHit,
) -> Result<(), String> {
    let record = hit.pass_record.as_check_record();
    let cache_hit_result = writer
        .emit_event(
            "info",
            "cache.hit",
            &[
                ("id", json!(record.id)),
                ("result", json!(record.result)),
                ("scope", json!(record.scope)),
                ("kind", json!(format!("{:?}", hit.kind))),
            ],
        )
        .map_err(|err| err.to_string());
    let mut diagnostic_log = Some(writer);
    // [w] Cached expectations still produce the same expectation.result
    // runtime-log events as evaluated expectations. Attempt that required
    // outcome independently so a cache.hit event error cannot suppress it.
    let result_event_result = write_expectation_result_event(&mut diagnostic_log, record);
    match (cache_hit_result, result_event_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(cache_error), Err(result_error)) => Err(format!(
            "{cache_error}; also failed to write cached expectation runtime log: {result_error}"
        )),
    }
}
