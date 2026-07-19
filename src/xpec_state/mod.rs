mod last_result;
mod retention;

use crate::check::{CheckRecord, ResolvedExpectation};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::state_paths::canon_state_path;
use crate::time::parse_record_timestamp;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[cfg(test)]
pub(crate) use last_result::LastResultResponse;
use last_result::{check_record_from_last_result, pass_record_from_cooldown_result};
pub(crate) use last_result::{LastResult, LastResultStatus};
pub(crate) use retention::{
    collected_expectation_ids_from_identities, prune_uncollected_xpec_state_dirs,
};

#[derive(Default)]
pub(crate) struct XpecStateCache {
    xpecs_dirs: BTreeMap<PathBuf, PathBuf>,
    xpec_dirs: BTreeMap<(PathBuf, String), PathBuf>,
    last_results: BTreeMap<LastResultCacheKey, Option<LastResult>>,
}

type LastResultCacheKey = (PathBuf, String, LastResultStatus);

pub(crate) struct CachedPassResultHit {
    // [uf] Both constructors below read only last-pass.json. Naming the payload
    // `last_pass` keeps fail history visibly outside the cached-result domain.
    pub(crate) last_pass: LastResult,
    pub(crate) kind: CachedPassResultKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CachedPassResultKind {
    SameTree,
    Cooldown,
}

impl XpecStateCache {
    pub(crate) fn bind_state_root(
        &mut self,
        project_root: &Path,
        state_root: &crate::state_paths::CanonStateRoot,
    ) {
        self.xpecs_dirs
            .insert(project_root.to_path_buf(), state_root.join("xpecs"));
    }

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
    // [uf] Cached Result is defined for an expectation and Git state. Same-tree
    // lookup compares the checked Git state with the last pass's stored
    // visible-tree OID; cooldown lookup consults that same last pass.
    if lookup.include_same_tree {
        if let Some(last_pass) = same_tree_last_pass_result(
            root,
            source,
            expectation,
            state_cache,
            visible_tree_oid_cache,
        )? {
            return Ok(Some(CachedPassResultHit {
                last_pass,
                kind: CachedPassResultKind::SameTree,
            }));
        }
    }
    if lookup.include_cooldown {
        if let Some(last_pass) =
            cooldown_last_pass_result(root, expectation, state_cache, lookup.now)?
        {
            return Ok(Some(CachedPassResultHit {
                last_pass,
                kind: CachedPassResultKind::Cooldown,
            }));
        }
    }
    Ok(None)
}

pub(crate) fn refresh_reused_same_tree_pass_result(
    root: &Path,
    source: &TreeSource,
    expectation: &ResolvedExpectation,
    state_cache: &mut XpecStateCache,
    mut hit: CachedPassResultHit,
) -> Result<CachedPassResultHit, String> {
    if hit.kind == CachedPassResultKind::SameTree {
        // The cached-result rule has already selected this hit. This write is
        // only the Last Results bookkeeping required when a same-tree result is
        // reused.
        let current_checked_tree_oid = source.tree_oid_for_prompt_diff(root)?;
        hit.last_pass = state_cache.refresh_git_backed_last_result_for_checked_tree(
            root,
            &current_checked_tree_oid,
            expectation,
            &hit.last_pass,
        )?;
    }
    Ok(hit)
}

pub(crate) fn check_record_from_cached_pass_result(
    root: &Path,
    expectation: &ResolvedExpectation,
    hit: &CachedPassResultHit,
) -> Result<CheckRecord, String> {
    match hit.kind {
        CachedPassResultKind::Cooldown => {
            pass_record_from_cooldown_result(root, expectation, &hit.last_pass)
        }
        CachedPassResultKind::SameTree => {
            check_record_from_last_result(root, expectation, &hit.last_pass)
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

fn same_tree_last_pass_result(
    root: &Path,
    source: &TreeSource,
    expectation: &ResolvedExpectation,
    state_cache: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Option<LastResult>, String> {
    let Some(last_pass) = state_cache.read_last_pass(root, expectation)? else {
        return Ok(None);
    };
    let resolver = visible_tree_oid_cache.stored_visible_scope_oid_resolver(root, source)?;
    result_matches_checked_tree(&resolver, &last_pass).map(|matches| matches.then_some(last_pass))
}

fn result_matches_checked_tree(
    resolver: &crate::git::StoredVisibleScopeOidResolver,
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
    let current_visible_tree_oid = resolver.oid_for_stored_visible_scope(&result.visible_scope)?;
    Ok(current_visible_tree_oid == stored_visible_tree_oid)
}

fn cooldown_last_pass_result(
    root: &Path,
    expectation: &ResolvedExpectation,
    state_cache: &mut XpecStateCache,
    now: u64,
) -> Result<Option<LastResult>, String> {
    // [uf] A cooldown result reuses only a recent last-pass result.
    let Some(cooldown) = expectation.cooldown else {
        return Ok(None);
    };
    let Some(last_pass) = state_cache.read_last_pass(root, expectation)? else {
        return Ok(None);
    };
    let Some(response_timestamp) = parse_record_timestamp(&last_pass.response_timestamp) else {
        return Ok(None);
    };
    if !response_is_within_cooldown(now, response_timestamp, cooldown.seconds) {
        return Ok(None);
    }
    Ok(Some(last_pass))
}

fn response_is_within_cooldown(now: u64, response_timestamp: u64, cooldown_seconds: u64) -> bool {
    // A response from the future is not an already-produced last pass and
    // therefore cannot become a cooldown result with an artificial age of 0.
    now.checked_sub(response_timestamp)
        .is_some_and(|age| age < cooldown_seconds)
}

#[cfg(test)]
mod tests {
    use super::response_is_within_cooldown;

    #[test] // xpec: uf
    fn cooldown_rejects_future_and_expired_response_timestamps() {
        assert!(!response_is_within_cooldown(100, 101, 10));
        assert!(!response_is_within_cooldown(100, 90, 10));
        assert!(response_is_within_cooldown(100, 91, 10));
    }
}
