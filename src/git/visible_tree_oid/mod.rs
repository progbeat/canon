use super::program::{head_tracked_files, staged_tracked_files, StagedTrackedFile};
use super::tree_source::TreeSource;
use crate::config_types::AgentConfig;
use crate::scope::{effective_ignore_patterns, path_bytes_in_scope, visible_scope};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod hash;
mod scope_entries;
#[cfg(test)]
mod tests;

use hash::{
    git_object_hash_algorithm, git_object_oid_hex_len, scope_entry_is_tree,
    visible_tree_oid_from_entries, GitObjectHashAlgorithm,
};
pub(crate) use hash::{git_object_oid_has_hex_len, git_object_oid_has_known_shape};
use scope_entries::{
    scope_includes_match_tracked_files, visible_scope_entries_from_files,
    visible_tree_oid_from_files_if_scope_present,
};

// Cache-spec ownership note: this module implements only the `visibleTreeOid`
// fingerprint. Answer-history storage, JSONL rendering, append, and compaction
// live under `history`, so whole Cache-spec review must inspect those modules
// in addition to this one.
type ScopeCacheKey = (PathBuf, Vec<String>, Vec<String>);
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
    gate_head_values: BTreeMap<ScopeCacheKey, Option<String>>,
    head_files: BTreeMap<PathBuf, Result<Option<Vec<StagedTrackedFile>>, String>>,
    object_hash_algorithms: BTreeMap<PathBuf, GitObjectHashAlgorithm>,
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

    pub(crate) fn repository_native_object_oid_hex_len(
        &mut self,
        root: &Path,
    ) -> Result<usize, String> {
        Ok(git_object_oid_hex_len(self.object_hash_algorithm(root)?))
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

    pub(crate) fn gate_head_tree_fingerprint(
        &mut self,
        root: &Path,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Option<String>, String> {
        let scope = visible_scope(agent, scope)?;
        cached_clone!(
            self.gate_head_values,
            source_scope_cache_key_parts(root, agent, &scope)?,
            |value| Ok(value),
            {
                let object_hash_algorithm = self.object_hash_algorithm(root)?;
                self.head_visible_scope_entries(root, &scope)?
                    .map(|entries| visible_tree_oid_from_entries(&entries, object_hash_algorithm))
                    .transpose()?
            },
            |value| Ok(value)
        )
    }

    fn head_visible_scope_entries(
        &mut self,
        root: &Path,
        scope: &[String],
    ) -> Result<Option<Vec<String>>, String> {
        let Some(files) = self.head_files(root)? else {
            return Ok(None);
        };
        if !scope_includes_match_tracked_files(&files, scope)? {
            return Ok(None);
        }
        visible_scope_entries_from_files(&files, scope).map(Some)
    }

    fn head_files(&mut self, root: &Path) -> Result<Option<Vec<StagedTrackedFile>>, String> {
        cached_clone!(
            self.head_files,
            root.to_path_buf(),
            |value| value,
            head_tracked_files(root),
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
