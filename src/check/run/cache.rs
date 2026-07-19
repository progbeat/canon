//! Cached results for the canon's expectation-and-Git-state domain.
//!
//! [uf,I4] In-place checks have no Git state and therefore cannot have cached
//! results. Their separate config contract rejects `cooldown`; this is the
//! domain boundary of the optional field, not a narrower cooldown syntax.

use crate::check::core::{CheckRecord, ResolvedExpectation};
use crate::check::interrogation::write_expectation_result_event;
use crate::config_types::AgentConfig;
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
    _agent: &AgentConfig,
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
    let hit = refresh_reused_same_tree_pass_result(root, source, expectation, xpec_state, hit)?;
    let record = check_record_from_cached_pass_result(root, expectation, &hit)?;
    // Human-review/error records are fail history for ordering, not
    // cached results. Treat any legacy record shaped that way as uncached so a
    // fresh evaluator run can produce the current-run ERROR output.
    if record.requires_human_review() {
        return Ok(None);
    }
    let kind = match hit.kind {
        CachedPassResultKind::SameTree => CachedResultKind::SameTree,
        CachedPassResultKind::Cooldown => CachedResultKind::Cooldown,
    };
    Ok(Some(CheckCacheHit { record, kind }))
}

pub(crate) fn write_cache_hit(
    writer: &mut DiagnosticLogWriter,
    hit: &CheckCacheHit,
) -> Result<(), String> {
    writer
        .emit_event(
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
    let mut diagnostic_log = Some(writer);
    // Cached expectations still produce the same expectation.result runtime-log
    // events as evaluated expectations, so logs expose every emitted
    // expectation outcome.
    write_expectation_result_event(&mut diagnostic_log, &hit.record)
}
