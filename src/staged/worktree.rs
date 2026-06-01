use crate::config_types::AgentConfig;
use crate::git::{GitBlobReader, StagedTrackedFile};
use crate::git::{TreeSource, VisibleTreeOidCache};
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
    remove_materialization_root_on_drop: bool,
    lazy_tree_dir: PathBuf,
    trees_dir: PathBuf,
    source_files: RefCell<Option<Vec<StagedTrackedFile>>>,
    unpacked_paths: RefCell<BTreeSet<Vec<u8>>>,
    blob_reader: RefCell<Option<GitBlobReader>>,
}

struct VisibleTree {
    oid: String,
    // The hardlink materialization spec names this `visible_tree.entry_paths`.
    // Each value is the evaluator-visible Git blob entry for that path, carrying
    // the mode and object id needed to extract and materialize the file.
    entry_paths: Vec<StagedTrackedFile>,
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
        let files = source.tracked_files(root)?;
        let snapshot_root = create_snapshot_root(root)?;
        let materialization_root = snapshot_root.path().to_path_buf();
        let remove_materialization_root_on_drop = snapshot_root.remove_on_drop();
        if let Err(err) = platform::create_private_dir_all(&materialization_root.join("lazy"))
            .and_then(|_| platform::create_private_dir_all(&materialization_root.join("trees")))
        {
            if remove_materialization_root_on_drop {
                let _ = fs::remove_dir_all(&materialization_root);
            }
            return Err(format!(
                "failed to initialize evaluator materialization root {}: {}",
                materialization_root.display(),
                err
            ));
        }
        Ok(StagedWorktreeView {
            source_root: root.to_path_buf(),
            source,
            remove_materialization_root_on_drop,
            lazy_tree_dir: materialization_root.join("lazy"),
            trees_dir: materialization_root.join("trees"),
            materialization_root,
            source_files: RefCell::new(Some(files)),
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
        visible_tree_oid: &str,
    ) -> Result<PathBuf, String> {
        let scope = sanitize_scope(scope, agent)?;
        let visible_tree = self.visible_tree(agent, &scope, visible_tree_oid)?;
        self.materialize_visible_tree(&visible_tree)
    }

    fn materialize_visible_tree(&self, visible_tree: &VisibleTree) -> Result<PathBuf, String> {
        // Lazy hardlink policy mapping:
        // - `unpack_missing_files` performs `git_tree.extract` once per
        //   not-yet-unpacked blob into `lazy_tree_dir`.
        // - `hardlink_visible_tree_root` builds the tree under
        //   `trees_dir/<visibleTreeOid>` and then chmods every directory
        //   read-only after its children are linked.
        // The visible tree OID is computed by the run-level VisibleTreeOidCache
        // before session start, so materialization does not start git per
        // expectation or per history-derived scope.
        let visible_tree_root = self.trees_dir.join(&visible_tree.oid);
        if visible_tree_root.exists() {
            return Ok(visible_tree_root);
        }
        self.unpack_missing_files(&visible_tree.entry_paths)?;
        self.hardlink_visible_tree_root(&visible_tree.entry_paths, visible_tree_root)
    }

    fn visible_tree(
        &self,
        agent: &AgentConfig,
        scope: &[String],
        visible_tree_oid: &str,
    ) -> Result<VisibleTree, String> {
        // Build the `visible_tree.entry_paths` set used by the hardlink
        // materialization policy. `TreeSource` supplies the checked Git tree
        // (staged index or explicit `--tree` revision); scope and ignore
        // filters define the evaluator-visible tree over its blob entries.
        let entry_paths = self
            .source_files()?
            .into_iter()
            .filter(|file| file.is_blob_file_entry())
            .filter(|file| path_bytes_in_scope(&file.path, scope))
            .filter(|file| !is_denied_path_bytes(agent, &file.path))
            .collect();
        Ok(VisibleTree {
            oid: visible_tree_oid.to_string(),
            entry_paths,
        })
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
            write_materialized_file(&self.lazy_tree_dir, file, &blob)?;
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

    fn hardlink_visible_tree_root(
        &self,
        files: &[StagedTrackedFile],
        visible_tree_root: PathBuf,
    ) -> Result<PathBuf, String> {
        if let Err(err) = platform::create_private_dir(&visible_tree_root) {
            if err.kind() == ErrorKind::AlreadyExists {
                return Ok(visible_tree_root);
            }
            return Err(format!(
                "failed to create evaluator visible tree root {}: {}",
                visible_tree_root.display(),
                err
            ));
        }
        for file in files {
            let relative = relative_path_from_git_path(&file.path)?;
            let source = self.lazy_tree_dir.join(&relative);
            let target = visible_tree_root.join(&relative);
            if let Some(parent) = target.parent() {
                platform::create_private_dir_all(parent).map_err(|err| {
                    format!(
                        "failed to create evaluator visible tree directory {}: {}",
                        parent.display(),
                        err
                    )
                })?;
            }
            platform::hardlink_file_or_copy_symlink(&source, &target)?;
        }
        // Equivalent to the policy's post-order dfs chmod: each directory is
        // made read-only only after every descendant has been created.
        make_visible_tree_directories_read_only(&visible_tree_root)?;
        Ok(visible_tree_root)
    }
}

impl Drop for StagedWorktreeView {
    fn drop(&mut self) {
        if self.remove_materialization_root_on_drop {
            let _ = make_materialization_tree_private(&self.trees_dir);
            let _ = fs::remove_dir_all(&self.materialization_root);
        }
    }
}

fn make_visible_tree_directories_read_only(path: &Path) -> Result<(), String> {
    for entry in fs::read_dir(path).map_err(|err| {
        format!(
            "failed to read evaluator visible tree directory {}: {}",
            path.display(),
            err
        )
    })? {
        let entry = entry.map_err(|err| {
            format!(
                "failed to read evaluator visible tree directory entry in {}: {}",
                path.display(),
                err
            )
        })?;
        let file_type = entry.file_type().map_err(|err| {
            format!(
                "failed to inspect evaluator scope path {}: {}",
                entry.path().display(),
                err
            )
        })?;
        if file_type.is_dir() {
            make_visible_tree_directories_read_only(&entry.path())?;
        }
    }
    platform::set_materialized_dir_permissions(path)
}

fn make_materialization_tree_private(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(format!(
                "failed to inspect evaluator materialization directory {}: {}",
                path.display(),
                err
            ));
        }
    };
    if !metadata.file_type().is_dir() {
        return platform::set_private_file_permissions(path);
    }
    platform::set_private_dir_permissions(path)?;
    for entry in fs::read_dir(path).map_err(|err| {
        format!(
            "failed to read evaluator materialization directory {}: {}",
            path.display(),
            err
        )
    })? {
        let entry = entry.map_err(|err| {
            format!(
                "failed to read evaluator materialization directory entry in {}: {}",
                path.display(),
                err
            )
        })?;
        make_materialization_tree_private(&entry.path())?;
    }
    Ok(())
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
    match fs::symlink_metadata(&target) {
        Ok(_) => fs::remove_file(&target).map_err(|err| {
            format!(
                "failed to replace evaluator lazy file {}: {}",
                target.display(),
                err
            )
        })?,
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => {
            return Err(format!(
                "failed to inspect evaluator lazy file {}: {}",
                target.display(),
                err
            ));
        }
    }
    if file.mode == "120000" {
        platform::create_materialized_symlink(blob, &target)?;
    } else {
        fs::write(&target, blob).map_err(|err| {
            format!(
                "failed to write evaluator lazy file {}: {}",
                target.display(),
                err
            )
        })?;
    }
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::config_types::AgentConfig;
    use crate::hash::full_scope;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::process;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn materialized_scope_files_and_directories_are_read_only() {
        let root = git_project("staged-snapshot-scope-read-only-dirs");
        fs::create_dir_all(root.join("dir")).unwrap();
        fs::write(root.join("dir/secret.txt"), "secret\n").unwrap();
        Command::new("git")
            .args(["add", "dir/secret.txt"])
            .current_dir(&root)
            .output()
            .unwrap();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let agent = empty_test_agent();
        let scope = full_scope();
        let staged_view = StagedWorktreeView::apply_with_visible_tree_oid_cache(
            &root,
            &mut visible_tree_oid_cache,
        )
        .unwrap();
        let visible_tree_oid = visible_tree_oid_cache
            .staged_visible_tree_oid(&root, &agent, &scope)
            .unwrap();
        let materialization_root = staged_view.materialization_root().to_path_buf();
        let scope_root = staged_view
            .materialize_evaluator_scope(&agent, &scope, &visible_tree_oid)
            .unwrap();

        assert_dir_mode(&materialization_root, 0o700);
        assert_dir_mode(&materialization_root.join("lazy"), 0o700);
        assert_dir_mode(&materialization_root.join("lazy/dir"), 0o700);
        assert_dir_mode(&materialization_root.join("trees"), 0o700);
        assert_file_mode(&materialization_root.join("lazy/dir/secret.txt"), 0o444);
        assert_dir_mode(&scope_root, 0o555);
        assert_dir_mode(&scope_root.join("dir"), 0o555);
        assert_file_mode(&scope_root.join("dir/secret.txt"), 0o444);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn materialized_symlink_permissions_do_not_follow_targets() {
        let root = git_project("staged-snapshot-symlink-no-follow");
        symlink("missing-target", root.join("link.txt")).unwrap();
        Command::new("git")
            .args(["add", "link.txt"])
            .current_dir(&root)
            .output()
            .unwrap();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let agent = empty_test_agent();
        let scope = full_scope();
        let staged_view = StagedWorktreeView::apply_with_visible_tree_oid_cache(
            &root,
            &mut visible_tree_oid_cache,
        )
        .unwrap();
        let visible_tree_oid = visible_tree_oid_cache
            .staged_visible_tree_oid(&root, &agent, &scope)
            .unwrap();
        let materialization_root = staged_view.materialization_root().to_path_buf();
        let scope_root = staged_view
            .materialize_evaluator_scope(&agent, &scope, &visible_tree_oid)
            .unwrap();

        assert_symlink_target(
            &materialization_root.join("lazy/link.txt"),
            Path::new("missing-target"),
        );
        assert_symlink_target(&scope_root.join("link.txt"), Path::new("missing-target"));

        let _ = fs::remove_dir_all(root);
    }

    fn empty_test_agent() -> AgentConfig {
        AgentConfig {
            models: Vec::new(),
            thinking: "medium".to_string(),
            ignore: Vec::new(),
            plugins: Vec::new(),
        }
    }

    fn assert_dir_mode(path: &Path, expected: u32) {
        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode,
            expected,
            "{} mode is {:o}, expected {:o}",
            path.display(),
            mode,
            expected
        );
    }

    fn assert_file_mode(path: &Path, expected: u32) {
        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode,
            expected,
            "{} mode is {:o}, expected {:o}",
            path.display(),
            mode,
            expected
        );
    }

    fn assert_symlink_target(path: &Path, expected: &Path) {
        let metadata = fs::symlink_metadata(path).unwrap();
        assert!(
            metadata.file_type().is_symlink(),
            "{} should be a symlink",
            path.display()
        );
        assert_eq!(fs::read_link(path).unwrap(), expected);
    }

    fn git_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("canon-test-{}-{}-{}", name, process::id(), unique));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Command::new("git")
            .arg("init")
            .current_dir(&root)
            .output()
            .unwrap();
        for args in [
            ["config", "core.autocrlf", "false"],
            ["config", "core.eol", "lf"],
        ] {
            let output = Command::new("git")
                .args(args)
                .current_dir(&root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git config failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        fs::write(root.join("README.md"), "hello").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        Command::new("git")
            .arg("add")
            .arg(".")
            .current_dir(&root)
            .output()
            .unwrap();
        root
    }
}
