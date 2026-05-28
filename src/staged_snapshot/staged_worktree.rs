use crate::config_types::AgentConfig;
use crate::git::{staged_tracked_files_for_pathspecs, GitBlobReader, StagedTrackedFile};
use crate::hash::full_scope;
use crate::platform;
use crate::scope::{
    effective_ignore_patterns, excluding_ignore_pathspec, sanitize_scope,
    scope_pathspecs_with_excludes,
};
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
    active_excludes_by_deny: RefCell<BTreeMap<Vec<String>, Vec<String>>>,
    unpacked_paths: RefCell<BTreeSet<Vec<u8>>>,
    blob_reader: RefCell<Option<GitBlobReader>>,
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
        Ok(StagedWorktreeView {
            source_root: root.to_path_buf(),
            lazy_root: materialization_root.join("lazy"),
            scope_roots: materialization_root.join("scopes"),
            materialization_root,
            active_excludes_by_deny: RefCell::new(BTreeMap::new()),
            unpacked_paths: RefCell::new(BTreeSet::new()),
            blob_reader: RefCell::new(None),
            next_scope_id: Cell::new(0),
        })
    }

    #[cfg(test)]
    pub(crate) fn materialization_root(&self) -> &Path {
        &self.materialization_root
    }

    pub(crate) fn materialize_evaluator_scope(
        &self,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<PathBuf, String> {
        let scope = sanitize_scope(scope, agent)?;
        let deny_patterns = sorted_effective_ignore_patterns(agent);
        // Configured ignore patterns are part of evaluator-visible Git tree
        // construction (see glossary). The lazy hardlink policy below receives
        // the already selected `scope_paths` for that tree.
        let scope_paths = self.scope_paths_in_evaluator_visible_git_tree(&scope, &deny_patterns)?;
        self.materialize_scope(&scope, &scope_paths)
    }

    fn materialize_scope(
        &self,
        scope: &[String],
        scope_paths: &[StagedTrackedFile],
    ) -> Result<PathBuf, String> {
        self.unpack_missing_files(scope_paths)?;
        if scope == full_scope() {
            return Ok(self.lazy_root.clone());
        }
        self.hardlink_scope_root(scope_paths)
    }

    fn scope_paths_in_evaluator_visible_git_tree(
        &self,
        scope: &[String],
        deny_patterns: &[String],
    ) -> Result<Vec<StagedTrackedFile>, String> {
        let active_excludes = self.active_excluding_ignore_pathspecs(deny_patterns)?;
        let pathspecs = scope_pathspecs_with_excludes(scope, &active_excludes);
        staged_tracked_files_for_pathspecs(&self.source_root, &pathspecs).map(|files| {
            files
                .into_iter()
                .filter(StagedTrackedFile::is_materialized_blob)
                .collect()
        })
    }

    fn active_excluding_ignore_pathspecs(
        &self,
        deny_patterns: &[String],
    ) -> Result<Vec<String>, String> {
        if let Some(active) = self.active_excludes_by_deny.borrow().get(deny_patterns) {
            return Ok(active.clone());
        }
        let mut active = Vec::new();
        for pattern in deny_patterns {
            if !staged_tracked_files_for_pathspecs(
                &self.source_root,
                std::slice::from_ref(pattern),
            )?
            .is_empty()
            {
                active.push(excluding_ignore_pathspec(pattern));
            }
        }
        self.active_excludes_by_deny
            .borrow_mut()
            .insert(deny_patterns.to_vec(), active.clone());
        Ok(active)
    }

    fn unpack_missing_files(&self, files: &[StagedTrackedFile]) -> Result<(), String> {
        let missing = {
            let unpacked = self.unpacked_paths.borrow();
            files
                .iter()
                .filter(|file| !unpacked.contains(&file.path))
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
            write_materialized_file(&self.lazy_root, file, &blob)?;
        }
        let mut unpacked = self.unpacked_paths.borrow_mut();
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

    fn hardlink_scope_root(&self, files: &[StagedTrackedFile]) -> Result<PathBuf, String> {
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
            let source = self.lazy_root.join(&relative);
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
            fs::hard_link(&source, &target).map_err(|err| {
                format!(
                    "failed to hardlink evaluator scope file {} to {}: {}",
                    source.display(),
                    target.display(),
                    err
                )
            })?;
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
