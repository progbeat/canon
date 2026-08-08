use super::last_result::{check_record_from_last_result, pass_record_from_cooldown_result};
use super::{LastResult, XpecStateCache};
use crate::check::{CheckRecord, ResolvedExpectation};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::time::parse_record_timestamp;
use std::path::Path;

pub(crate) struct CachedPassResultHit {
    // [m] Both constructors below read only last-pass.json. Naming the payload
    // `last_pass` keeps fail history visibly outside the cached-result domain.
    pub(crate) last_pass: LastResult,
    pub(crate) kind: CachedPassResultKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedPassResultKind {
    SameTree,
    Cooldown,
}

pub(crate) struct CachedPassResultLookup {
    pub(crate) now: u64,
    pub(crate) include_same_tree: bool,
    pub(crate) include_cooldown: bool,
}

pub(crate) fn cached_pass_result_for_expectation(
    root: &Path,
    source: &TreeSource,
    expectation: &ResolvedExpectation,
    state_cache: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    lookup: CachedPassResultLookup,
) -> Result<Option<CachedPassResultHit>, String> {
    // [m] Cached Result is defined for an expectation and Git state. Same-tree
    // lookup compares the checked Git state with the last pass's stored
    // visible-tree OID; cooldown lookup consults that same last pass.
    if !lookup.include_same_tree && !lookup.include_cooldown {
        return Ok(None);
    }
    let Some(last_pass) = state_cache.read_last_pass(root, expectation)? else {
        return Ok(None);
    };
    cached_pass_result_for_stored_last_pass(
        root,
        source,
        expectation,
        last_pass,
        visible_tree_oid_cache,
        lookup,
    )
}

fn cached_pass_result_for_stored_last_pass(
    root: &Path,
    source: &TreeSource,
    expectation: &ResolvedExpectation,
    last_pass: LastResult,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    lookup: CachedPassResultLookup,
) -> Result<Option<CachedPassResultHit>, String> {
    if lookup.include_same_tree
        && stored_visible_scope_matches_checked_tree(
            root,
            source,
            &last_pass.visible_scope,
            last_pass.visible_tree_oid.as_deref(),
            visible_tree_oid_cache,
        )?
    {
        return Ok(Some(CachedPassResultHit {
            last_pass,
            kind: CachedPassResultKind::SameTree,
        }));
    }
    if lookup.include_cooldown
        && response_timestamp_is_within_cooldown(
            expectation,
            &last_pass.response_timestamp,
            lookup.now,
        )
    {
        // [m,90] Cooldown deliberately does not inspect checkedTreeOid. The
        // canonical rule selects the latest pass solely by configured duration
        // and response timestamp; in-place mode prevents lookup at its command
        // boundary rather than assigning a different meaning to stored passes.
        return Ok(Some(CachedPassResultHit {
            last_pass,
            kind: CachedPassResultKind::Cooldown,
        }));
    }
    Ok(None)
}

pub(crate) fn refresh_reused_same_tree_pass_result(
    root: &Path,
    current_checked_tree_oid: &str,
    expectation: &ResolvedExpectation,
    state_cache: &mut XpecStateCache,
    mut hit: CachedPassResultHit,
) -> Result<CachedPassResultHit, String> {
    if hit.kind == CachedPassResultKind::SameTree {
        // The cached-result rule has already selected this hit. This write is
        // only the Last Results bookkeeping required when a same-tree result is
        // reused.
        hit.last_pass = state_cache.refresh_existing_git_backed_pass_for_checked_tree(
            root,
            current_checked_tree_oid,
            expectation,
        )?;
    }
    Ok(hit)
}

pub(crate) fn check_record_from_cached_pass_result(
    root: &Path,
    expectation: &ResolvedExpectation,
    hit: &CachedPassResultHit,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<CheckRecord, String> {
    match hit.kind {
        CachedPassResultKind::Cooldown => pass_record_from_cooldown_result(
            root,
            expectation,
            &hit.last_pass,
            visible_tree_oid_cache,
        ),
        CachedPassResultKind::SameTree => {
            check_record_from_last_result(root, expectation, &hit.last_pass, visible_tree_oid_cache)
        }
    }
}

pub(crate) fn latest_fail_timestamp(
    root: &Path,
    expectation: &ResolvedExpectation,
    last_result_history: &mut XpecStateCache,
) -> Result<Option<u64>, String> {
    // Reading a last-fail timestamp is history lookup for ordering. Unlike
    // `cached_pass_result_for_expectation`, it cannot produce a reusable
    // result or remove the expectation from the evaluation queue.
    Ok(last_result_history
        .read_last_fail(root, expectation)?
        .and_then(|result| parse_record_timestamp(&result.response_timestamp)))
}

pub(crate) fn stored_visible_scope_matches_checked_tree(
    root: &Path,
    source: &TreeSource,
    visible_scope: &[String],
    stored_visible_tree_oid: Option<&str>,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<bool, String> {
    let Some(stored_visible_tree_oid) = stored_visible_tree_oid else {
        return Ok(false);
    };
    // The cached-result rule compares the stored visibleTreeOid with the
    // current visible tree built from that same stored visible-scope pathspec.
    // `src/git/visible_tree_oid` owns the scoped-tree OID calculation reused
    // here for historical visible scopes.
    // Reconstructing a q-scope here would make current agent ignores part of
    // history reuse.
    let resolver = visible_tree_oid_cache.stored_visible_scope_oid_resolver(root, source)?;
    let current_visible_tree_oid = resolver.oid_for_stored_visible_scope(visible_scope)?;
    Ok(current_visible_tree_oid == stored_visible_tree_oid)
}

pub(crate) fn response_timestamp_is_within_cooldown(
    expectation: &ResolvedExpectation,
    response_timestamp: &str,
    now: u64,
) -> bool {
    // [m] A cooldown result reuses only a recent last-pass result.
    let Some(cooldown) = expectation.cooldown else {
        return false;
    };
    let Some(response_timestamp) = parse_record_timestamp(response_timestamp) else {
        return false;
    };
    response_is_within_cooldown(now, response_timestamp, cooldown.seconds)
}

fn response_is_within_cooldown(now: u64, response_timestamp: u64, cooldown_seconds: u64) -> bool {
    // A response from the future is not an already-produced last pass and
    // therefore cannot become a cooldown result with an artificial age of 0.
    now.checked_sub(response_timestamp)
        .is_some_and(|age| age < cooldown_seconds)
}
