use super::program::{staged_tracked_files, StagedTrackedFile};
use super::tree_source::TreeSource;
use crate::config_types::AgentConfig;
use crate::scope::{effective_ignore_patterns, path_bytes_in_scope, visible_scope};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod hash;
mod scope_entries;
#[cfg(test)]
mod tests;

pub(crate) use hash::git_object_oid_has_known_shape;
use hash::{git_object_hash_algorithm, scope_entry_is_tree, GitObjectHashAlgorithm};
use scope_entries::{
    visible_scope_entries_from_files, visible_tree_oid_from_files_if_scope_present,
};

// This module implements only the `visibleTreeOid` fingerprint. Persistent
// per-expectation result state lives under `xpec_state`.
type SourceScopeCacheKey = (PathBuf, String, Vec<String>, Vec<String>);
type SourceFilesCacheKey = (PathBuf, String);

macro_rules! cached_clone {
    ($cache:expr, $key:expr, |$cached:ident| $hit:expr, $compute:expr, |$computed:ident| $miss:expr) => {{
        let key = $key;
        if let Some($cached) = $cache.get(&key).cloned() {
            return $hit;
        }
        let $computed = $compute;
        $cache.insert(key, $computed.clone());
        $miss
    }};
}

#[derive(Default)]
pub(crate) struct VisibleTreeOidCache {
    visible_tree_oids: BTreeMap<SourceScopeCacheKey, Option<String>>,
    visible_scope_entries: BTreeMap<SourceScopeCacheKey, Vec<String>>,
    tree_source_files: BTreeMap<SourceFilesCacheKey, Result<Vec<StagedTrackedFile>, String>>,
    staged_files: BTreeMap<PathBuf, Result<Vec<StagedTrackedFile>, String>>,
    object_hash_algorithms: BTreeMap<PathBuf, GitObjectHashAlgorithm>,
}

pub(crate) struct VisibleTreeOidReuseResolver {
    agent: AgentConfig,
    files: Vec<StagedTrackedFile>,
    object_hash_algorithm: GitObjectHashAlgorithm,
}

impl VisibleTreeOidCache {
    pub(crate) fn new() -> VisibleTreeOidCache {
        VisibleTreeOidCache::default()
    }

    pub(crate) fn visible_tree_oid(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<String, String> {
        self.visible_tree_oid_for_reuse(root, source, agent, scope)?
            .ok_or("failed to hash tree scope".to_string())
    }

    pub(crate) fn visible_tree_oid_for_reuse(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Option<String>, String> {
        let scope = visible_scope(agent, scope)?;
        self.visible_tree_oid_for_source_scope(root, source, agent, scope)
    }

    pub(crate) fn reuse_resolver(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
    ) -> Result<VisibleTreeOidReuseResolver, String> {
        // History reuse can scan many records with many stored scopes. Snapshot
        // the source file list and hash algorithm once; each per-record scope
        // is filtered and hashed in-process, so distinct history scopes do not
        // start additional Git subprocesses.
        Ok(VisibleTreeOidReuseResolver {
            agent: agent.clone(),
            files: self.files_for_source(root, source)?,
            object_hash_algorithm: self.object_hash_algorithm(root)?,
        })
    }

    pub(crate) fn checked_file_count(
        &mut self,
        root: &Path,
        source: &TreeSource,
    ) -> Result<usize, String> {
        self.files_for_source(root, source).map(|files| files.len())
    }

    pub(crate) fn visible_file_count(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<usize, String> {
        let scope = visible_scope(agent, scope)?;
        let entries = self.visible_scope_entries_for_source(root, source, agent, &scope)?;
        Ok(entries
            .iter()
            .filter(|entry| !scope_entry_is_tree(entry))
            .count())
    }

    pub(crate) fn visible_scope_intersects_pathspecs(
        &mut self,
        root: &Path,
        source: &TreeSource,
        visible_scope: &[String],
        pathspecs: &[String],
    ) -> Result<bool, String> {
        for file in self.files_for_source(root, source)? {
            if path_bytes_in_scope(&file.path, visible_scope)?
                && path_bytes_in_scope(&file.path, pathspecs)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn visible_tree_oid_for_source_scope(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        scope: Vec<String>,
    ) -> Result<Option<String>, String> {
        cached_clone!(
            self.visible_tree_oids,
            source_scope_cache_key(root, source, agent, &scope)?,
            |value| Ok(value),
            {
                let files = self.files_for_source(root, source)?;
                visible_tree_oid_from_files_if_scope_present(
                    &files,
                    &scope,
                    self.object_hash_algorithm(root)?,
                )?
            },
            |value| Ok(value)
        )
    }

    fn visible_scope_entries_for_source(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Vec<String>, String> {
        cached_clone!(
            self.visible_scope_entries,
            source_scope_cache_key(root, source, agent, scope)?,
            |value| Ok(value),
            {
                // Keep direct git subprocesses independent of the number of
                // scopes or history records: list each tree source once, then
                // filter scopes here.
                let files = self.files_for_source(root, source)?;
                visible_scope_entries_from_files(&files, scope)?
            },
            |value| Ok(value)
        )
    }

    fn files_for_source(
        &mut self,
        root: &Path,
        source: &TreeSource,
    ) -> Result<Vec<StagedTrackedFile>, String> {
        match source {
            TreeSource::Staged => self.staged_files(root),
            TreeSource::Git { .. } => self.tree_source_files(root, source),
        }
    }

    fn staged_files(&mut self, root: &Path) -> Result<Vec<StagedTrackedFile>, String> {
        cached_clone!(
            self.staged_files,
            root.to_path_buf(),
            |value| value,
            staged_tracked_files(root),
            |value| value
        )
    }

    fn tree_source_files(
        &mut self,
        root: &Path,
        source: &TreeSource,
    ) -> Result<Vec<StagedTrackedFile>, String> {
        cached_clone!(
            self.tree_source_files,
            (root.to_path_buf(), source.cache_key()),
            |value| value,
            source.tracked_files(root),
            |value| value
        )
    }

    fn object_hash_algorithm(&mut self, root: &Path) -> Result<GitObjectHashAlgorithm, String> {
        cached_clone!(
            self.object_hash_algorithms,
            root.to_path_buf(),
            |value| Ok(value),
            git_object_hash_algorithm(root)?,
            |value| Ok(value)
        )
    }
}

impl VisibleTreeOidReuseResolver {
    pub(crate) fn visible_tree_oid_for_scope(
        &self,
        scope: &[String],
    ) -> Result<Option<String>, String> {
        let scope = visible_scope(&self.agent, scope)?;
        visible_tree_oid_from_files_if_scope_present(
            &self.files,
            &scope,
            self.object_hash_algorithm,
        )
    }
}

fn source_scope_cache_key_parts(
    root: &Path,
    agent: &AgentConfig,
    scope: &[String],
) -> Result<(PathBuf, Vec<String>, Vec<String>), String> {
    let mut ignore_patterns = effective_ignore_patterns(agent)?;
    ignore_patterns.sort();
    ignore_patterns.dedup();
    Ok((root.to_path_buf(), scope.to_vec(), ignore_patterns))
}

fn source_scope_cache_key(
    root: &Path,
    source: &TreeSource,
    agent: &AgentConfig,
    scope: &[String],
) -> Result<SourceScopeCacheKey, String> {
    let (root, scope, ignore_patterns) = source_scope_cache_key_parts(root, agent, scope)?;
    Ok((root, source.cache_key(), scope, ignore_patterns))
}
