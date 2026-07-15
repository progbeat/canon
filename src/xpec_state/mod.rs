mod cleanup;
mod last_result;

use crate::check::{CheckRecord, ResolvedExpectation};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::state_paths::canon_state_path;
use crate::time::parse_record_timestamp;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(crate) use cleanup::{active_expectation_ids_from_identities, cleanup_stale_xpec_dirs};
use last_result::{check_record_from_last_result, pass_record_from_cooldown_result};
pub(crate) use last_result::{LastResult, LastResultStatus};

#[derive(Default)]
pub(crate) struct XpecStateCache {
    xpecs_dirs: BTreeMap<PathBuf, PathBuf>,
    xpec_dirs: BTreeMap<(PathBuf, String), PathBuf>,
    last_results: BTreeMap<LastResultCacheKey, Option<LastResult>>,
}

type LastResultCacheKey = (PathBuf, String, LastResultStatus);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedResultStatus {
    Pass,
}

pub(crate) struct CachedLastResultHit {
    pub(crate) result: LastResult,
    pub(crate) status: CachedResultStatus,
    pub(crate) kind: CachedLastResultKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedLastResultKind {
    SameTree,
    Cooldown,
}

impl XpecStateCache {
    pub(crate) fn xpecs_dir(&mut self, root: &Path) -> Result<PathBuf, String> {
        let key = root.to_path_buf();
        if let Some(path) = self.xpecs_dirs.get(&key) {
            return Ok(path.clone());
        }
        let path = canon_state_path(root, "xpecs")?;
        self.xpecs_dirs.insert(key, path.clone());
        Ok(path)
    }

    pub(crate) fn xpec_dir(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
    ) -> Result<PathBuf, String> {
        let key = (root.to_path_buf(), expectation.id.clone());
        if let Some(path) = self.xpec_dirs.get(&key) {
            return Ok(path.clone());
        }
        let path = self.xpecs_dir(root)?.join(&expectation.id);
        self.xpec_dirs.insert(key, path.clone());
        Ok(path)
    }
}

pub(crate) fn snapshot_pass_ids(
    root: &Path,
    expectations: &[ResolvedExpectation],
    cache: &mut XpecStateCache,
) -> Result<BTreeSet<String>, String> {
    let mut ids = BTreeSet::new();
    for expectation in expectations {
        if cache.read_last_pass(root, expectation)?.is_some() {
            ids.insert(expectation.id.clone());
        }
    }
    Ok(ids)
}

pub(crate) struct CachedLastResultLookup {
    pub(crate) now: u64,
    pub(crate) include_same_tree: bool,
    pub(crate) include_cooldown: bool,
}

pub(crate) fn cached_last_result_for_expectation(
    root: &Path,
    source: &TreeSource,
    expectation: &ResolvedExpectation,
    state_cache: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    lookup: CachedLastResultLookup,
) -> Result<Option<CachedLastResultHit>, String> {
    // Cached Result is defined for an expectation and Git state. Same-tree
    // lookup compares the checked Git state with the last pass's stored
    // visible-tree OID; cooldown lookup consults that same last pass.
    if lookup.include_same_tree {
        if let Some(result) = same_tree_last_result(
            root,
            source,
            expectation,
            state_cache,
            visible_tree_oid_cache,
        )? {
            return Ok(Some(CachedLastResultHit {
                result,
                status: CachedResultStatus::Pass,
                kind: CachedLastResultKind::SameTree,
            }));
        }
    }
    if lookup.include_cooldown {
        if let Some(result) = cooldown_last_result(root, expectation, state_cache, lookup.now)? {
            return Ok(Some(CachedLastResultHit {
                result,
                status: CachedResultStatus::Pass,
                kind: CachedLastResultKind::Cooldown,
            }));
        }
    }
    Ok(None)
}

pub(crate) fn refresh_reused_same_tree_last_result(
    root: &Path,
    source: &TreeSource,
    expectation: &ResolvedExpectation,
    state_cache: &mut XpecStateCache,
    mut hit: CachedLastResultHit,
) -> Result<CachedLastResultHit, String> {
    if hit.kind == CachedLastResultKind::SameTree {
        // The cached-result rule has already selected this hit. This write is
        // only the Last Results bookkeeping required when a same-tree result is
        // reused.
        let current_checked_tree_oid = source.tree_oid_for_prompt_diff(root)?;
        hit.result = state_cache.refresh_last_result_for_checked_tree(
            root,
            &current_checked_tree_oid,
            expectation,
            &hit.result,
        )?;
    }
    Ok(hit)
}

pub(crate) fn check_record_from_cached_result(
    root: &Path,
    expectation: &ResolvedExpectation,
    hit: &CachedLastResultHit,
) -> Result<CheckRecord, String> {
    match hit.status {
        CachedResultStatus::Pass if hit.kind == CachedLastResultKind::Cooldown => {
            pass_record_from_cooldown_result(root, expectation, &hit.result)
        }
        CachedResultStatus::Pass => check_record_from_last_result(root, expectation, &hit.result),
    }
}

pub(crate) fn latest_fail_timestamp(
    root: &Path,
    expectation: &ResolvedExpectation,
    cache: &mut XpecStateCache,
) -> Result<Option<u64>, String> {
    Ok(cache
        .read_last_fail(root, expectation)?
        .and_then(|result| parse_record_timestamp(&result.response_timestamp)))
}

fn same_tree_last_result(
    root: &Path,
    source: &TreeSource,
    expectation: &ResolvedExpectation,
    state_cache: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Option<LastResult>, String> {
    let Some(result) = state_cache.read_last_pass(root, expectation)? else {
        return Ok(None);
    };
    let resolver = visible_tree_oid_cache.reuse_resolver(root, source)?;
    result_matches_checked_tree(&resolver, &result).map(|matches| matches.then_some(result))
}

fn result_matches_checked_tree(
    resolver: &crate::git::VisibleTreeOidReuseResolver,
    result: &LastResult,
) -> Result<bool, String> {
    let Some(stored_visible_tree_oid) = result.visible_tree_oid.as_deref() else {
        return Ok(false);
    };
    // The cached-result rule compares the stored visibleTreeOid with the
    // current visible tree built from that same stored visible-scope pathspec.
    // `src/git/visible_tree_oid` owns the scoped-tree OID calculation reused
    // here for historical visible scopes.
    // Reconstructing a q-scope here would make current agent ignores part of
    // history reuse.
    let Some(current_visible_tree_oid) =
        resolver.visible_tree_oid_for_visible_scope_pathspec(&result.visible_scope)?
    else {
        return Ok(false);
    };
    Ok(current_visible_tree_oid == stored_visible_tree_oid)
}

fn cooldown_last_result(
    root: &Path,
    expectation: &ResolvedExpectation,
    state_cache: &mut XpecStateCache,
    now: u64,
) -> Result<Option<LastResult>, String> {
    let Some(cooldown) = expectation.cooldown else {
        return Ok(None);
    };
    let Some(result) = state_cache.read_last_pass(root, expectation)? else {
        return Ok(None);
    };
    let Some(response_timestamp) = parse_record_timestamp(&result.response_timestamp) else {
        return Ok(None);
    };
    if now.saturating_sub(response_timestamp) >= cooldown.seconds {
        return Ok(None);
    }
    Ok(Some(result))
}
