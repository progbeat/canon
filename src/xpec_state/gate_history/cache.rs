use super::super::{LastResult, LastResultStatus, XpecStateCache};
use super::migration::{
    git_backed_fail_from_last_result, git_backed_pass_from_last_result, preserve_canonical_results,
    validate_git_backed_result,
};
use super::model::{GateHistory, GitBackedFail, GitBackedPass};
use super::persistence::{load_history_file, persist_cache_path, CACHE_FILE_NAME};
use crate::check::ResolvedExpectation;
use std::path::{Path, PathBuf};

impl XpecStateCache {
    pub(crate) fn read_gate_results(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
    ) -> Result<Option<GateHistory>, String> {
        let Some(mut cache) = self.cached_or_load_history(root, expectation)? else {
            return Ok(None);
        };
        self.merge_canonical_results(root, expectation, &mut cache)?;
        Ok(Some(cache))
    }

    pub(in crate::xpec_state) fn write_git_backed_last_result(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
        result: &LastResult,
    ) -> Result<(), String> {
        // [fh,KD,cw] Recording callers have retained their complete
        // configuration; the gate caller can only refresh an existing pass.
        // In-place results have no checked tree OID and never call this writer.
        validate_git_backed_result(result)?;
        let mut cache = self
            .cached_or_load_history(root, expectation)?
            .unwrap_or_default();
        // [KD] Canonical Git-backed status files supersede stale or missing
        // cache halves. Canonical in-place results do not: they intentionally
        // omit Git tree identity and cannot replace a known Git-backed half.
        self.merge_canonical_results(root, expectation, &mut cache)?;
        match result.status {
            LastResultStatus::Pass => {
                cache.last_pass = Some(GitBackedPass {
                    response_timestamp: result.response_timestamp.clone(),
                    visible_scope: result.visible_scope.clone(),
                    visible_tree_oid: result
                        .visible_tree_oid
                        .clone()
                        .expect("validated Git-backed pass has visibleTreeOid"),
                });
            }
            LastResultStatus::Fail => {
                cache.last_fail = Some(GitBackedFail {
                    checked_tree_oid: result
                        .checked_tree_oid
                        .clone()
                        .expect("validated Git-backed result has checkedTreeOid"),
                });
            }
        }
        self.persist_cache(root, expectation, cache)
    }

    pub(in crate::xpec_state) fn preserve_gate_results_before_in_place_update(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
    ) -> Result<(), String> {
        // [KD,90] On first use after upgrading from canonical-only Last
        // Results, an in-place write must not erase the only Git-backed copy.
        let xpec_dir = self.xpec_dir(root, expectation)?;
        preserve_canonical_results(&xpec_dir)?;
        let id = expectation.require_configured_id()?.to_string();
        self.gate_results.remove(&(root.to_path_buf(), id));
        Ok(())
    }

    fn persist_cache(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
        cache: GateHistory,
    ) -> Result<(), String> {
        let path = self.cache_path(root, expectation)?;
        persist_cache_path(&path, &cache)?;
        let id = expectation.require_configured_id()?.to_string();
        self.gate_results
            .insert((root.to_path_buf(), id), Some(cache));
        Ok(())
    }

    fn cached_or_load_history(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
    ) -> Result<Option<GateHistory>, String> {
        let id = expectation.require_configured_id()?.to_string();
        let key = (root.to_path_buf(), id);
        // [d] An invocation-local hit performs no filesystem access. On a miss,
        // load_history_file validates the path immediately before reading it.
        if let Some(cache) = self.gate_results.get(&key) {
            return Ok(cache.clone());
        }
        let path = self.cache_path(root, expectation)?;
        let cache = match load_history_file(&path)? {
            Some(cache) => cache,
            None => {
                self.gate_results.insert(key, None);
                return Ok(None);
            }
        };
        self.gate_results.insert(key, Some(cache.clone()));
        Ok(Some(cache))
    }

    fn merge_canonical_results(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
        cache: &mut GateHistory,
    ) -> Result<(), String> {
        if let Some(last_pass) = self
            .read_last_pass(root, expectation)?
            .and_then(git_backed_pass_from_last_result)
        {
            cache.last_pass = Some(last_pass);
        }
        if let Some(last_fail) = self
            .read_last_fail(root, expectation)?
            .and_then(git_backed_fail_from_last_result)
        {
            cache.last_fail = Some(last_fail);
        }
        Ok(())
    }

    fn cache_path(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
    ) -> Result<PathBuf, String> {
        Ok(self.xpec_dir(root, expectation)?.join(CACHE_FILE_NAME))
    }
}
