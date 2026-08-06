use super::{
    prune_uncollected_in_place_xpec_state, prune_uncollected_xpec_state_dirs, GateHistory,
    LastResult, LastResultStatus,
};
use crate::check::{ExpectationIdentity, ResolvedExpectation};
use crate::state_paths::canon_state_path;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

// [d,fh] This invocation-local owner memoizes resolved paths and parsed records,
// and records which complete configuration has successfully passed retention.
// Persistent writers require that retained-configuration capability, so no
// normal write path can bypass pruning the current mode's obsolete per-ID
// state.
#[derive(Default)]
pub(crate) struct XpecStateCache {
    pub(super) xpecs_dirs: BTreeMap<PathBuf, PathBuf>,
    pub(super) xpec_dirs: BTreeMap<(PathBuf, String), PathBuf>,
    pub(super) last_results: BTreeMap<LastResultCacheKey, Option<LastResult>>,
    pub(super) gate_results: BTreeMap<(PathBuf, String), Option<GateHistory>>,
    in_place_bindings: BTreeSet<PathBuf>,
    retained_configuration_full_ids: BTreeMap<PathBuf, BTreeSet<String>>,
}

pub(super) type LastResultCacheKey = (PathBuf, String, LastResultStatus);

impl XpecStateCache {
    pub(crate) fn bind_state_root(
        &mut self,
        project_root: &Path,
        state_root: &crate::state_paths::CanonStateRoot,
    ) {
        self.reset_project_binding(project_root);
        self.in_place_bindings.remove(project_root);
        self.xpecs_dirs.insert(
            project_root.to_path_buf(),
            state_root.join(crate::state_paths::XPEC_STATE_DIR_NAME),
        );
    }

    pub(crate) fn bind_in_place_state_root(
        &mut self,
        project_root: &Path,
        state_root: &crate::state_paths::CanonStateRoot,
    ) {
        // [KD,90] In-place writes the canonical per-ID Last Results, but its
        // working-directory configuration does not own the Git-backed result
        // cache consumed by gate.
        self.bind_state_root(project_root, state_root);
        self.in_place_bindings.insert(project_root.to_path_buf());
    }

    fn reset_project_binding(&mut self, project_root: &Path) {
        self.retained_configuration_full_ids.remove(project_root);
        self.xpec_dirs.retain(|(root, _), _| root != project_root);
        self.last_results
            .retain(|(root, _, _), _| root != project_root);
        self.gate_results
            .retain(|(root, _), _| root != project_root);
    }

    pub(crate) fn retain_only_current_configuration(
        &mut self,
        root: &Path,
        identities: &[ExpectationIdentity],
    ) -> Result<(usize, usize), String> {
        // [fh,Ijl,KD] The component first prunes state outside the complete
        // current full-ID set. In-place retention keeps only bounded Git-backed
        // gate data for absent IDs. Only a successful sweep grants this
        // invocation's later last-result writers permission to persist.
        let current_full_ids = identities
            .iter()
            .map(|identity| identity.id.clone())
            .collect::<BTreeSet<_>>();
        self.retained_configuration_full_ids.remove(root);
        let xpecs_dir = self.xpecs_dir(root)?;
        let retention = if self.in_place_bindings.contains(root) {
            prune_uncollected_in_place_xpec_state(&xpecs_dir, &current_full_ids)?
        } else {
            prune_uncollected_xpec_state_dirs(&xpecs_dir, &current_full_ids)?
        };
        self.retained_configuration_full_ids
            .insert(root.to_path_buf(), current_full_ids);
        Ok((retention.removed, retention.kept))
    }

    pub(super) fn require_retained_expectation(
        &self,
        root: &Path,
        expectation: &ResolvedExpectation,
    ) -> Result<(), String> {
        let id = expectation.require_configured_id()?;
        match self.retained_configuration_full_ids.get(root) {
            Some(ids) if ids.contains(id) => Ok(()),
            _ => Err(format!(
                "cannot write xpec state for `{id}` before retaining the current configuration"
            )),
        }
    }

    pub(super) fn require_retained_configuration(
        &self,
        root: &Path,
        identities: &[ExpectationIdentity],
    ) -> Result<(), String> {
        let current_full_ids = identities
            .iter()
            .map(|identity| identity.id.clone())
            .collect::<BTreeSet<_>>();
        match self.retained_configuration_full_ids.get(root) {
            Some(retained) if retained == &current_full_ids => Ok(()),
            _ => Err(
                "cannot write check failure history before retaining the current configuration"
                    .to_string(),
            ),
        }
    }

    pub(super) fn xpecs_dir(&mut self, root: &Path) -> Result<PathBuf, String> {
        let key = root.to_path_buf();
        if let Some(path) = self.xpecs_dirs.get(&key) {
            return Ok(path.clone());
        }
        let path = canon_state_path(root, crate::state_paths::XPEC_STATE_DIR_NAME)?;
        self.xpecs_dirs.insert(key, path.clone());
        Ok(path)
    }

    pub(super) fn xpec_dir(
        &mut self,
        root: &Path,
        expectation: &ResolvedExpectation,
    ) -> Result<PathBuf, String> {
        let id = expectation.require_configured_id()?.to_string();
        let key = (root.to_path_buf(), id.clone());
        if let Some(path) = self.xpec_dirs.get(&key) {
            return Ok(path.clone());
        }
        let path = self.xpecs_dir(root)?.join(&id);
        self.xpec_dirs.insert(key, path.clone());
        Ok(path)
    }
}
