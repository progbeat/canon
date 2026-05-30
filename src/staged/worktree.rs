use crate::config_types::AgentConfig;
use crate::git::tree_source::TreeSource;
use crate::git::visible_tree_oid::{visible_tree_oid_for_tracked_files, VisibleTreeOidCache};
use crate::git::{GitBlobReader, StagedTrackedFile};
use crate::platform;
use crate::scope::{is_denied_path_bytes, path_bytes_in_scope, sanitize_scope};
use crate::staged::paths::create_snapshot_root;
#[cfg(test)]
pub(crate) use crate::staged::paths::snapshot_parent_outside_worktree;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub(crate) struct StagedWorktreeView {
    source_root: PathBuf,
    source: TreeSource,
    materialization_root: PathBuf,
    lazy_root: PathBuf,
    scope_roots: PathBuf,
    source_files: RefCell<Option<Vec<StagedTrackedFile>>>,
    unpacked_paths: RefCell<BTreeSet<Vec<u8>>>,
    blob_reader: RefCell<Option<GitBlobReader>>,
}

impl StagedWorktreeView {
    #[cfg(test)]
    pub(crate) fn apply(root: &Path) -> Result<StagedWorktreeView, String> {
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        StagedWorktreeView::apply_with_visible_tree_oid_cache(root, &mut visible_tree_oid_cache)
    }

    #[cfg(test)]
    pub(crate) fn apply_with_visible_tree_oid_cache(
        root: &Path,
        visible_tree_oid_cache: &mut VisibleTreeOidCache,
    ) -> Result<StagedWorktreeView, String> {
        StagedWorktreeView::apply_for_tree_source(root, TreeSource::Staged, visible_tree_oid_cache)
    }

    pub(crate) fn apply_for_tree_source(
        root: &Path,
        source: TreeSource,
        _visible_tree_oid_cache: &mut VisibleTreeOidCache,
    ) -> Result<StagedWorktreeView, String> {
        let _files = source.tracked_files(root)?;
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
            source,
            lazy_root: materialization_root.join("lazy"),
            scope_roots: materialization_root.join("scopes"),
            materialization_root,
            source_files: RefCell::new(None),
            unpacked_paths: RefCell::new(BTreeSet::new()),
            blob_reader: RefCell::new(None),
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
        let scope_paths = self.evaluator_visible_git_tree(agent, &scope)?;
        self.materialize_scope(&scope_paths)
    }

    fn materialize_scope(&self, scope_paths: &[StagedTrackedFile]) -> Result<PathBuf, String> {
        let scope_root = self.scope_root_for_files(scope_paths)?;
        if scope_root.exists() {
            return Ok(scope_root);
        }
        self.unpack_missing_files(scope_paths)?;
        self.hardlink_scope_root(scope_paths, scope_root)
    }

    fn evaluator_visible_git_tree(
        &self,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Vec<StagedTrackedFile>, String> {
        // The lazy hardlink policy is defined over blob-backed
        // file_entries(git_tree). Canon's TreeSource supplies that Git tree,
        // whether it is the staged index tree or an explicit `--tree`
        // revision, then the materializer applies visible-scope and ignore
        // filters to those file entries.
        Ok(self
            .source_files()?
            .into_iter()
            .filter(|file| file.is_blob_file_entry())
            .filter(|file| path_bytes_in_scope(&file.path, scope))
            .filter(|file| !is_denied_path_bytes(agent, &file.path))
            .collect())
    }

    fn source_files(&self) -> Result<Vec<StagedTrackedFile>, String> {
        if let Some(files) = self.source_files.borrow().as_ref() {
            return Ok(files.clone());
        }
        let files = self.source.tracked_files(&self.source_root)?;
        *self.source_files.borrow_mut() = Some(files.clone());
        Ok(files)
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

    fn scope_root_for_files(&self, files: &[StagedTrackedFile]) -> Result<PathBuf, String> {
        let scope_tree_oid = visible_tree_oid_for_tracked_files(&self.source_root, files)?;
        Ok(self.scope_roots.join(scope_tree_oid))
    }

    fn hardlink_scope_root(
        &self,
        files: &[StagedTrackedFile],
        scope_root: PathBuf,
    ) -> Result<PathBuf, String> {
        if let Err(err) = platform::create_private_dir(&scope_root) {
            if err.kind() == ErrorKind::AlreadyExists {
                return Ok(scope_root);
            }
            return Err(format!(
                "failed to create evaluator scope root {}: {}",
                scope_root.display(),
                err
            ));
        }
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
            platform::hardlink_file_or_copy_symlink(&source, &target)?;
        }
        Ok(scope_root)
    }
}

impl Drop for StagedWorktreeView {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.materialization_root);
    }
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
    if file.mode == "120000" {
        return platform::create_materialized_symlink(blob, &target);
    }
    fs::write(&target, blob).map_err(|err| {
        format!(
            "failed to write evaluator lazy file {}: {}",
            target.display(),
            err
        )
    })?;
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
