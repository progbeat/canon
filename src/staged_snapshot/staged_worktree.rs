use crate::config_types::AgentConfig;
use crate::git::{staged_tracked_files, GitBlobReader, StagedTrackedFile};
use crate::hash::full_scope;
use crate::platform;
use crate::scope::{effective_ignore_patterns, path_matches_pattern_bytes, sanitize_scope};
use crate::staged_worktree_paths::create_snapshot_root;
#[cfg(test)]
pub(crate) use crate::staged_worktree_paths::snapshot_parent_outside_worktree;
use crate::visible_tree_oid::VisibleTreeOidCache;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) struct StagedWorktreeView {
    source_root: PathBuf,
    materialization_root: PathBuf,
    lazy_root: PathBuf,
    scope_roots: PathBuf,
    files: Vec<StagedTrackedFile>,
    lazy_trees_by_deny: RefCell<BTreeMap<Vec<String>, PathBuf>>,
    unpacked_paths_by_deny: RefCell<BTreeMap<Vec<String>, BTreeSet<Vec<u8>>>>,
    blob_reader: RefCell<Option<GitBlobReader>>,
    next_lazy_id: Cell<u64>,
    next_scope_id: Cell<u64>,
}

impl StagedWorktreeView {
    #[cfg(test)]
    pub(crate) fn apply(root: &Path) -> Result<StagedWorktreeView, String> {
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        StagedWorktreeView::apply_with_visible_tree_oid_cache(root, &mut visible_tree_oid_cache)
    }

    pub(crate) fn apply_with_visible_tree_oid_cache(
        root: &Path,
        _visible_tree_oid_cache: &mut VisibleTreeOidCache,
    ) -> Result<StagedWorktreeView, String> {
        let materialization_root = create_snapshot_root(root)?;
        if let Err(err) = platform::create_private_dir(&materialization_root.join("lazy"))
            .and_then(|_| platform::create_private_dir(&materialization_root.join("scopes")))
        {
            let _ = fs::remove_dir_all(&materialization_root);
            return Err(format!(
                "failed to initialize evaluator materialization root {}: {}",
                materialization_root.display(),
                err
            ));
        }
        let files = match staged_tracked_files(root) {
            Ok(files) => files,
            Err(err) => {
                let _ = fs::remove_dir_all(&materialization_root);
                return Err(err);
            }
        };
        Ok(StagedWorktreeView {
            source_root: root.to_path_buf(),
            lazy_root: materialization_root.join("lazy"),
            scope_roots: materialization_root.join("scopes"),
            materialization_root,
            files,
            lazy_trees_by_deny: RefCell::new(BTreeMap::new()),
            unpacked_paths_by_deny: RefCell::new(BTreeMap::new()),
            blob_reader: RefCell::new(None),
            next_lazy_id: Cell::new(0),
            next_scope_id: Cell::new(0),
        })
    }

    #[cfg(test)]
    pub(crate) fn materialization_root(&self) -> &Path {
        &self.materialization_root
    }

    pub(crate) fn materialize_scope(
        &self,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<PathBuf, String> {
        let scope = sanitize_scope(scope, agent)?;
        let deny_patterns = sorted_effective_ignore_patterns(agent);
        let visible_files = self.visible_files(&scope, &deny_patterns);
        let lazy_tree = self.lazy_tree_for_deny_patterns(&deny_patterns)?;
        self.unpack_missing_files(&lazy_tree, &deny_patterns, &visible_files)?;
        self.copy_scope_root(&lazy_tree, &visible_files)
    }

    fn visible_files(&self, scope: &[String], deny_patterns: &[String]) -> Vec<StagedTrackedFile> {
        // Materialization uses the same glossary visible-scope order as
        // visibleTreeOid hashing: include files from the stored q-scope/full
        // scope first, then apply normalized ignore patterns as exclusions.
        self.files
            .iter()
            .filter(|file| {
                file.is_materialized_blob()
                    && path_is_in_scope(&file.path, scope)
                    && !deny_patterns
                        .iter()
                        .any(|pattern| path_matches_pattern_bytes(&file.path, pattern.as_bytes()))
            })
            .cloned()
            .collect()
    }

    fn lazy_tree_for_deny_patterns(&self, deny_patterns: &[String]) -> Result<PathBuf, String> {
        if let Some(root) = self.lazy_trees_by_deny.borrow().get(deny_patterns) {
            return Ok(root.clone());
        }
        let id = self.next_lazy_id.get();
        self.next_lazy_id.set(id + 1);
        let root = self.lazy_root.join(id.to_string());
        platform::create_private_dir(&root).map_err(|err| {
            format!(
                "failed to create evaluator lazy tree {}: {}",
                root.display(),
                err
            )
        })?;
        self.lazy_trees_by_deny
            .borrow_mut()
            .insert(deny_patterns.to_vec(), root.clone());
        Ok(root)
    }

    fn unpack_missing_files(
        &self,
        lazy_tree: &Path,
        deny_patterns: &[String],
        files: &[StagedTrackedFile],
    ) -> Result<(), String> {
        let missing = {
            let unpacked_by_deny = self.unpacked_paths_by_deny.borrow();
            let unpacked = unpacked_by_deny.get(deny_patterns);
            files
                .iter()
                .filter(|file| !unpacked.is_some_and(|paths| paths.contains(&file.path)))
                .cloned()
                .collect::<Vec<_>>()
        };
        if missing.is_empty() {
            return Ok(());
        }

        let object_ids = missing
            .iter()
            .map(|file| file.object_id.clone())
            .collect::<Vec<_>>();
        let blobs = self.read_missing_blobs(&object_ids)?;
        for (file, blob) in missing.iter().zip(blobs) {
            write_materialized_file(lazy_tree, file, &blob)?;
        }
        let mut unpacked_by_deny = self.unpacked_paths_by_deny.borrow_mut();
        let unpacked = unpacked_by_deny.entry(deny_patterns.to_vec()).or_default();
        for file in missing {
            unpacked.insert(file.path);
        }
        Ok(())
    }

    fn read_missing_blobs(&self, object_ids: &[String]) -> Result<Vec<Vec<u8>>, String> {
        let mut reader = self.blob_reader.borrow_mut();
        if reader.is_none() {
            *reader = Some(GitBlobReader::new(&self.source_root)?);
        }
        reader
            .as_mut()
            .expect("git blob reader was initialized")
            .read_blobs(object_ids)
    }

    fn copy_scope_root(
        &self,
        lazy_tree: &Path,
        files: &[StagedTrackedFile],
    ) -> Result<PathBuf, String> {
        let id = self.next_scope_id.get();
        self.next_scope_id.set(id + 1);
        let scope_root = self.scope_roots.join(id.to_string());
        platform::create_private_dir(&scope_root).map_err(|err| {
            format!(
                "failed to create evaluator scope root {}: {}",
                scope_root.display(),
                err
            )
        })?;
        for file in files {
            let relative = relative_path_from_git_path(&file.path)?;
            let source = lazy_tree.join(&relative);
            let target = scope_root.join(&relative);
            if let Some(parent) = target.parent() {
                platform::create_private_dir_all(parent).map_err(|err| {
                    format!(
                        "failed to create evaluator scope directory {}: {}",
                        parent.display(),
                        err
                    )
                })?;
            }
            fs::copy(&source, &target).map_err(|err| {
                format!(
                    "failed to copy evaluator scope file {} to {}: {}",
                    source.display(),
                    target.display(),
                    err
                )
            })?;
            platform::set_materialized_file_permissions(&target, &file.mode)?;
        }
        Ok(scope_root)
    }
}

impl Drop for StagedWorktreeView {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.materialization_root);
    }
}

fn sorted_effective_ignore_patterns(agent: &AgentConfig) -> Vec<String> {
    let mut patterns = effective_ignore_patterns(agent);
    patterns.sort();
    patterns.dedup();
    patterns
}

fn path_is_in_scope(path: &[u8], scope: &[String]) -> bool {
    scope == full_scope()
        || scope
            .iter()
            .any(|base| path_components_start_with(path, base.as_bytes()))
}

fn path_components_start_with(path: &[u8], base: &[u8]) -> bool {
    let path_parts = path_components(path);
    let base_parts = path_components(base);
    !base_parts.is_empty() && path_parts.starts_with(&base_parts)
}

fn path_components(path: &[u8]) -> Vec<&[u8]> {
    path.split(|byte| *byte == b'/')
        .filter(|component| !component.is_empty())
        .collect()
}

fn write_materialized_file(
    lazy_tree: &Path,
    file: &StagedTrackedFile,
    blob: &[u8],
) -> Result<(), String> {
    let relative = relative_path_from_git_path(&file.path)?;
    let target = lazy_tree.join(relative);
    if let Some(parent) = target.parent() {
        platform::create_private_dir_all(parent).map_err(|err| {
            format!(
                "failed to create evaluator lazy directory {}: {}",
                parent.display(),
                err
            )
        })?;
    }
    fs::write(&target, blob).map_err(|err| {
        format!(
            "failed to write evaluator lazy file {}: {}",
            target.display(),
            err
        )
    })?;
    // Git mode 120000 stores symlink targets as blob bytes. The lazy
    // materialization policy intentionally writes those bytes as a regular
    // file so evaluator reads cannot follow links outside the staged tree.
    platform::set_materialized_file_permissions(&target, &file.mode)
}

fn relative_path_from_git_path(path: &[u8]) -> Result<PathBuf, String> {
    let path = PathBuf::from(platform::os_string_from_bytes(path.to_vec())?);
    if path.is_absolute() {
        return Err(format!(
            "staged file path must be relative: {}",
            path.display()
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!(
            "staged file path must not contain '..': {}",
            path.display()
        ));
    }
    Ok(path)
}
