#[cfg(test)]
use crate::config_types::AgentConfig;
use crate::git::git_object_oid_has_known_shape;
use crate::git::TreeSource;
#[cfg(test)]
use crate::git::VisibleTreeOidCache;
use crate::git::{GitBlobReader, StagedTrackedFile};
use crate::platform;
use crate::scope::path_bytes_in_scope;
use crate::staged::paths::create_snapshot_root;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
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
    // The hardlink materialization spec names this `visible_tree.entry_paths`:
    // Git entries selected by applying the visible scope pathspec to the
    // checked tree. Blob-backed entries can be extracted into the lazy tree;
    // gitlinks are represented as empty directories in the materialized view.
    entry_paths: Vec<StagedTrackedFile>,
}

struct VisibleTreeChild {
    name: Vec<u8>,
    path: Vec<u8>,
    is_dir: bool,
}

impl StagedWorktreeView {
    pub(crate) fn apply_for_tree_source(
        root: &Path,
        source: TreeSource,
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

    pub(crate) fn materialize_visible_scope(
        &self,
        visible_scope: &[String],
        visible_tree_oid: &str,
    ) -> Result<PathBuf, String> {
        let visible_tree = self.visible_tree(visible_scope, visible_tree_oid)?;
        self.materialize_visible_tree(&visible_tree)
    }

    fn materialize_visible_tree(&self, visible_tree: &VisibleTree) -> Result<PathBuf, String> {
        // Lazy hardlink policy mapping:
        // - `unpack_missing_visible_entries` walks every not-yet-unpacked
        //   visible entry. Blob-backed entries are extracted into
        //   `lazy_tree_dir`; gitlinks are marked handled here and created as
        //   directories by `hardlink_visible_tree_root`.
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
        self.unpack_missing_visible_entries(visible_tree)?;
        self.hardlink_visible_tree_root(visible_tree, visible_tree_root)
    }

    fn visible_tree(
        &self,
        scope: &[String],
        visible_tree_oid: &str,
    ) -> Result<VisibleTree, String> {
        if !git_object_oid_has_known_shape(visible_tree_oid) {
            return Err("visibleTreeOid must be a Git object ID hex string".to_string());
        }
        // Build the `visible_tree.entry_paths` set used by the hardlink
        // materialization policy. `TreeSource` supplies the checked Git tree
        // (staged index or explicit `--tree` revision); the visible scope
        // pathspec defines the evaluator-visible tree.
        let mut entry_paths = Vec::new();
        for file in self.source_files()? {
            if path_bytes_in_scope(&file.path, scope)? {
                entry_paths.push(file);
            }
        }
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

    fn unpack_missing_visible_entries(&self, visible_tree: &VisibleTree) -> Result<(), String> {
        let (missing_blobs, missing_non_blobs) = {
            let unpacked = self.unpacked_paths.borrow();
            let missing = visible_tree
                .entry_paths
                .iter()
                .filter(|file| !unpacked.contains(&file.path))
                .cloned()
                .collect::<Vec<_>>();
            let missing_blobs = missing
                .iter()
                .filter(|file| file.is_blob_file_entry())
                .cloned()
                .collect::<Vec<_>>();
            let missing_non_blobs = missing
                .into_iter()
                .filter(|file| !file.is_blob_file_entry())
                .collect::<Vec<_>>();
            (missing_blobs, missing_non_blobs)
        };
        for file in missing_non_blobs {
            extract_non_blob_visible_entry(&self.lazy_tree_dir, &file)?;
            self.unpacked_paths.borrow_mut().insert(file.path);
        }
        if missing_blobs.is_empty() {
            return Ok(());
        }

        let object_ids = missing_blobs
            .iter()
            .map(|file| file.object_id.clone())
            .collect::<Vec<_>>();
        let blobs = self.read_missing_blobs(&object_ids)?;
        for (file, blob) in missing_blobs.into_iter().zip(blobs) {
            write_materialized_file(&self.lazy_tree_dir, &file, &blob)?;
            self.unpacked_paths.borrow_mut().insert(file.path);
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
        visible_tree: &VisibleTree,
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
        self.hardlink_visible_tree_children(visible_tree, b"", &visible_tree_root)?;
        remove_write_permissions_from_materialized_dir(&visible_tree_root)?;
        Ok(visible_tree_root)
    }

    fn hardlink_visible_tree_children(
        &self,
        visible_tree: &VisibleTree,
        prefix: &[u8],
        target_dir: &Path,
    ) -> Result<(), String> {
        for child in visible_tree.children(prefix) {
            let target = target_dir.join(platform::os_string_from_bytes(child.name)?);
            if child.is_dir {
                platform::create_private_dir(&target).map_err(|err| {
                    format!(
                        "failed to create evaluator visible tree directory {}: {}",
                        target.display(),
                        err
                    )
                })?;
                self.hardlink_visible_tree_children(visible_tree, &child.path, &target)?;
                remove_write_permissions_from_materialized_dir(&target)?;
            } else {
                let source = self
                    .lazy_tree_dir
                    .join(relative_path_from_git_path(&child.path)?);
                platform::hardlink_file_or_copy_symlink(&source, &target)?;
            }
        }
        Ok(())
    }
}

impl VisibleTree {
    fn children(&self, prefix: &[u8]) -> Vec<VisibleTreeChild> {
        let prefix_components = path_components(prefix);
        let mut children = BTreeMap::new();
        for file in &self.entry_paths {
            let components = path_components(&file.path);
            if !components.starts_with(&prefix_components)
                || components.len() == prefix_components.len()
            {
                continue;
            }
            let child_components = &components[..prefix_components.len() + 1];
            let child_path = join_path_components(child_components);
            let is_leaf = child_components.len() == components.len();
            let is_dir = !is_leaf || !file.is_blob_file_entry();
            children
                .entry(child_path.clone())
                .or_insert_with(|| VisibleTreeChild {
                    name: child_components
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .to_vec(),
                    path: child_path,
                    is_dir,
                });
        }
        children.into_values().collect()
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

fn path_components(path: &[u8]) -> Vec<&[u8]> {
    path.split(|byte| *byte == b'/')
        .filter(|component| !component.is_empty())
        .collect()
}

fn join_path_components(components: &[&[u8]]) -> Vec<u8> {
    let mut path = Vec::new();
    for component in components {
        if !path.is_empty() {
            path.push(b'/');
        }
        path.extend_from_slice(component);
    }
    path
}

fn extract_non_blob_visible_entry(
    lazy_tree: &Path,
    file: &StagedTrackedFile,
) -> Result<(), String> {
    let relative = relative_path_from_git_path(&file.path)?;
    let target = lazy_tree.join(&relative);
    platform::create_private_dir_all(&target).map_err(|err| {
        format!(
            "failed to create evaluator lazy directory {}: {}",
            target.display(),
            err
        )
    })?;
    remove_write_permissions_from_materialized_dir(&target)
}

fn remove_write_permissions_from_materialized_dir(path: &Path) -> Result<(), String> {
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
    let target = lazy_tree.join(&relative);
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
    remove_write_permissions_from_extracted_file(&target, &file.mode)
}

fn remove_write_permissions_from_extracted_file(path: &Path, mode: &str) -> Result<(), String> {
    // The platform helper opens symlinks with `ChmodSymlink::Ignore`, matching
    // the policy's `follow_symlinks=False` requirement.
    platform::set_materialized_file_permissions(path, mode)
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
        let visible_scope = full_scope();
        let staged_view =
            StagedWorktreeView::apply_for_tree_source(&root, TreeSource::Staged).unwrap();
        let visible_tree_oid = visible_tree_oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &agent, &scope)
            .unwrap();
        let scope_root = staged_view
            .materialize_visible_scope(&visible_scope, &visible_tree_oid)
            .unwrap();

        assert_dir_read_only(&scope_root);
        assert_dir_read_only(&scope_root.join("dir"));
        assert_file_read_only(&scope_root.join("dir/secret.txt"));

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
        let visible_scope = full_scope();
        let staged_view =
            StagedWorktreeView::apply_for_tree_source(&root, TreeSource::Staged).unwrap();
        let visible_tree_oid = visible_tree_oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &agent, &scope)
            .unwrap();
        let scope_root = staged_view
            .materialize_visible_scope(&visible_scope, &visible_tree_oid)
            .unwrap();

        assert_symlink_target(&scope_root.join("link.txt"), Path::new("missing-target"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn materialization_rejects_non_oid_tree_root_name() {
        let root = git_project("staged-snapshot-reject-tree-root-escape");
        let visible_scope = full_scope();
        let staged_view =
            StagedWorktreeView::apply_for_tree_source(&root, TreeSource::Staged).unwrap();
        let escape = root.join("escape-root");
        let err = staged_view
            .materialize_visible_scope(&visible_scope, &escape.to_string_lossy())
            .unwrap_err();

        assert!(err.contains("visibleTreeOid"));
        assert!(!escape.exists());
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

    fn assert_dir_read_only(path: &Path) {
        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_ne!(mode & 0o555, 0, "{} should be readable", path.display());
        assert_eq!(mode & 0o222, 0, "{} should not be writable", path.display());
    }

    fn assert_file_read_only(path: &Path) {
        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_ne!(mode & 0o444, 0, "{} should be readable", path.display());
        assert_eq!(mode & 0o222, 0, "{} should not be writable", path.display());
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
