mod cleanup;
mod last_result;

#[cfg(test)]
mod tests;

use crate::check::{CheckRecord, CheckResult, SelectedExpectation};
use crate::config_types::AgentConfig;
use crate::git::{resolve_git_path, TreeSource, VisibleTreeOidCache};
use crate::scope::q_scope_from_visible_scope;
use crate::state_paths::CANON_XPECS_DIR_GIT_PATH;
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
    Fail,
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
        let path = resolve_git_path(root, CANON_XPECS_DIR_GIT_PATH)?;
        self.xpecs_dirs.insert(key, path.clone());
        Ok(path)
    }

    pub(crate) fn xpec_dir(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<PathBuf, String> {
        let key = (root.to_path_buf(), expectation.id.clone());
        if let Some(path) = self.xpec_dirs.get(&key) {
            return Ok(path.clone());
        }
        let path = self.xpecs_dir(root)?.join(&expectation.id);
        self.xpec_dirs.insert(key, path.clone());
        Ok(path)
    }

    pub(crate) fn read_stored_q_scope(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<Option<Vec<String>>, String> {
        Ok([
            LastResultStatus::Pass,
            LastResultStatus::Fail,
            LastResultStatus::Error,
        ]
        .into_iter()
        .map(|status| self.read_last_result(root, expectation, status))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .max_by_key(|result| parse_record_timestamp(&result.updated_timestamp).unwrap_or(0))
        .map(|result| result.q_scope))
    }

    fn latest_answer_result(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<Option<LastResult>, String> {
        Ok([LastResultStatus::Pass, LastResultStatus::Fail]
            .into_iter()
            .map(|status| self.read_last_result(root, expectation, status))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .max_by_key(|result| parse_record_timestamp(&result.updated_timestamp).unwrap_or(0)))
    }
}

pub(crate) fn snapshot_pass_ids(
    root: &Path,
    expectations: &[SelectedExpectation],
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
    expectation: &SelectedExpectation,
    state_cache: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    lookup: CachedLastResultLookup,
) -> Result<Option<CachedLastResultHit>, String> {
    // Cached results are answers for the checked visible tree. `diff-from`
    // only chooses the left-hand tree for prompt-rendered Git diffs during
    // fresh evaluator work, so it is not part of cache identity.
    if lookup.include_same_tree {
        if let Some((result, status)) = same_tree_last_result(
            root,
            source,
            &expectation.agent,
            expectation,
            state_cache,
            visible_tree_oid_cache,
        )? {
            return Ok(Some(CachedLastResultHit {
                result,
                status,
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
    expectation: &SelectedExpectation,
    state_cache: &mut XpecStateCache,
    mut hit: CachedLastResultHit,
) -> Result<CachedLastResultHit, String> {
    if hit.kind == CachedLastResultKind::SameTree {
        // The cached-result rule has already selected this hit. This write is
        // only the Last Results bookkeeping required when a same-tree result is
        // reused.
        hit.result = state_cache.refresh_last_result(root, expectation, &hit.result)?;
    }
    Ok(hit)
}

pub(crate) fn check_record_from_cached_result(
    expectation: &SelectedExpectation,
    hit: &CachedLastResultHit,
) -> CheckRecord {
    match hit.status {
        CachedResultStatus::Pass if hit.kind == CachedLastResultKind::Cooldown => {
            pass_record_from_cooldown_result(expectation, &hit.result)
        }
        CachedResultStatus::Pass | CachedResultStatus::Fail => {
            check_record_from_last_result(expectation, &hit.result)
        }
    }
}

pub(crate) fn latest_non_pass_timestamp(
    root: &Path,
    expectation: &SelectedExpectation,
    cache: &mut XpecStateCache,
) -> Result<Option<u64>, String> {
    // Human-review results are persisted as `last-error.json`: evaluator
    // schema errors such as ScopeTooNarrow are not pass/fail answers, and
    // `CheckRecord::requires_human_review` is defined by the same `error`
    // field. Ordering therefore treats fail and error status files as the
    // complete non-pass history.
    let fail = cache.read_last_fail(root, expectation)?;
    let error = cache.read_last_error(root, expectation)?;
    Ok([fail, error]
        .into_iter()
        .flatten()
        .filter_map(|result| parse_record_timestamp(&result.response_timestamp))
        .max())
}

fn same_tree_last_result(
    root: &Path,
    source: &TreeSource,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    state_cache: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Option<(LastResult, CachedResultStatus)>, String> {
    let resolver = visible_tree_oid_cache.reuse_resolver(root, source, agent)?;
    for (last_status, cached_status) in [
        (LastResultStatus::Fail, CachedResultStatus::Fail),
        (LastResultStatus::Pass, CachedResultStatus::Pass),
    ] {
        let Some(result) = state_cache.read_last_result(root, expectation, last_status)? else {
            continue;
        };
        let Some(stored_visible_tree_oid) = result.visible_tree_oid.as_deref() else {
            continue;
        };
        let Ok(q_scope) = q_scope_from_visible_scope(agent, &result.visible_scope) else {
            continue;
        };
        let Some(current_visible_tree_oid) = resolver.visible_tree_oid_for_scope(&q_scope)? else {
            continue;
        };
        if current_visible_tree_oid == stored_visible_tree_oid {
            return Ok(Some((result, cached_status)));
        }
    }
    Ok(None)
}

fn cooldown_last_result(
    root: &Path,
    expectation: &SelectedExpectation,
    state_cache: &mut XpecStateCache,
    now: u64,
) -> Result<Option<LastResult>, String> {
    let Some(cooldown) = expectation.cooldown else {
        return Ok(None);
    };
    let latest = state_cache.latest_answer_result(root, expectation)?;
    let Some(result) = latest else {
        return Ok(None);
    };
    let Some(response_timestamp) = parse_record_timestamp(&result.response_timestamp) else {
        return Ok(None);
    };
    let check_result = match result.status {
        LastResultStatus::Pass => CheckResult::Pass,
        LastResultStatus::Fail => CheckResult::Fail,
        LastResultStatus::Error => return Ok(None),
    };
    let Some(duration) = cooldown.duration_for(check_result) else {
        return Ok(None);
    };
    if now.saturating_sub(response_timestamp) >= duration {
        return Ok(None);
    }
    Ok(Some(result))
}
