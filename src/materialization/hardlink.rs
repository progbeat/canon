use super::{
    entry_path::relative_path_from_git_path,
    permissions::remove_write_permissions_from_materialized_dir, visible_tree::VisibleTree,
    TreeMaterializer,
};
use crate::platform::filesystem;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

// Materializes evaluator input according to the hardlink materialization
// policy. setup.rs owns the filesystem-shaped project input; extract.rs
// materializes missing checked files and removes their write permissions; this
// module reuses complete visible input trees, hardlinks files or copies symlinks
// into a new tree, and removes write permissions from each materialized
// directory.
impl TreeMaterializer {
    pub(crate) fn visible_tree_root_path(&self, visible_tree_oid: &str) -> PathBuf {
        self.materialized_input
            .visible_trees_dir
            .join(visible_tree_oid)
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
        let visible_tree_root = self.visible_tree_root_path(&visible_tree.oid);
        if visible_tree_root.exists() {
            return Ok(visible_tree_root);
        }
        self.unpack_missing_visible_entries(visible_tree)?;
        self.hardlink_visible_tree_root(visible_tree, visible_tree_root)
    }

    fn hardlink_visible_tree_root(
        &self,
        visible_tree: &VisibleTree,
        visible_tree_root: PathBuf,
    ) -> Result<PathBuf, String> {
        let materializing_root = self.create_materializing_tree_root(&visible_tree.oid)?;
        let built = self
            .hardlink_visible_tree_children(visible_tree, b"", &materializing_root)
            .and_then(|()| remove_write_permissions_from_materialized_dir(&materializing_root));
        if let Err(err) = built {
            return Err(cleanup_materializing_tree_root(&materializing_root, err));
        }

        // [1t] Publish only a complete, read-only tree. A concurrent invocation
        // can observe either no OID root or the finished root, never a directory
        // that this invocation is still populating.
        if let Err(err) = fs::rename(&materializing_root, &visible_tree_root) {
            if visible_tree_root.exists() {
                super::remove_materialization_path(&materializing_root).map_err(|cleanup_err| {
                    format!(
                        "failed to remove redundant evaluator visible tree {} after another \
                         invocation published {}: {}",
                        materializing_root.display(),
                        visible_tree_root.display(),
                        cleanup_err
                    )
                })?;
                return Ok(visible_tree_root);
            }
            let publish_error = format!(
                "failed to publish evaluator visible tree {} as {}: {}",
                materializing_root.display(),
                visible_tree_root.display(),
                err
            );
            return Err(cleanup_materializing_tree_root(
                &materializing_root,
                publish_error,
            ));
        }
        if let Some(journal) = &self.materialized_input.rollback_journal {
            journal
                .borrow_mut()
                .created_visible_tree_roots
                .insert(visible_tree_root.clone());
        }
        Ok(visible_tree_root)
    }

    fn create_materializing_tree_root(&self, visible_tree_oid: &str) -> Result<PathBuf, String> {
        for _ in 0..64 {
            let random = getrandom::u64()
                .map_err(|err| format!("failed to choose visible tree staging path: {err}"))?;
            let path = self.materialized_input.visible_trees_dir.join(format!(
                ".canon-materializing-{}-{visible_tree_oid}-{random:016x}",
                std::process::id()
            ));
            match filesystem::create_private_dir(&path) {
                Ok(()) => return Ok(path),
                Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
                Err(err) => {
                    return Err(format!(
                        "failed to create evaluator visible tree staging root {}: {}",
                        path.display(),
                        err
                    ));
                }
            }
        }
        Err(format!(
            "failed to allocate evaluator visible tree staging root under {}",
            self.materialized_input.visible_trees_dir.display()
        ))
    }

    fn hardlink_visible_tree_children(
        &self,
        visible_tree: &VisibleTree,
        prefix: &[u8],
        target_dir: &Path,
    ) -> Result<(), String> {
        for child in visible_tree.children(prefix) {
            let target = target_dir.join(filesystem::os_string_from_bytes(child.name)?);
            if child.is_dir {
                filesystem::create_private_dir(&target).map_err(|err| {
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
                    .materialized_input
                    .extracted_files_dir
                    .join(relative_path_from_git_path(&child.path)?);
                filesystem::hardlink_file_or_copy_symlink(&source, &target)?;
            }
        }
        Ok(())
    }
}

fn cleanup_materializing_tree_root(path: &Path, primary_error: String) -> String {
    match super::remove_materialization_path(path) {
        Ok(()) => primary_error,
        Err(cleanup_error) => format!(
            "{}; failed to remove incomplete evaluator visible tree {}: {}",
            primary_error,
            path.display(),
            cleanup_error
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{assert_read_only, empty_test_agent, git_project};
    use super::*;
    use crate::git::TreeSource;
    use crate::git::VisibleTreeOidCache;
    use crate::hash::full_scope;
    use std::process::Command;

    #[test] // xpec: 1t
    fn failed_materialization_can_be_retried_after_its_input_recovers() {
        let root = git_project("staged-snapshot-retry-after-missing-blob");
        fs::write(root.join("file.txt"), "contents\n").unwrap();
        let output = Command::new("git")
            .args(["add", "file.txt"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let object_id_output = Command::new("git")
            .args(["rev-parse", ":file.txt"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            object_id_output.status.success(),
            "git rev-parse failed: {}",
            String::from_utf8_lossy(&object_id_output.stderr)
        );
        let object_id = String::from_utf8(object_id_output.stdout)
            .unwrap()
            .trim()
            .to_string();
        let object_path = root
            .join(".git/objects")
            .join(&object_id[..2])
            .join(&object_id[2..]);
        let withheld_object_path = object_path.with_extension("withheld");
        let scope = full_scope();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let visible_tree_oid = visible_tree_oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &empty_test_agent(), &scope)
            .unwrap();
        let tree_materializer =
            TreeMaterializer::apply_for_tree_source(&root, TreeSource::Staged).unwrap();
        fs::rename(&object_path, &withheld_object_path).unwrap();
        tree_materializer
            .materialize_visible_scope(&scope, &visible_tree_oid)
            .expect_err("missing Git blob must fail materialization");
        fs::rename(&withheld_object_path, &object_path).unwrap();
        let scope_root = tree_materializer
            .materialize_visible_scope(&scope, &visible_tree_oid)
            .unwrap();

        assert_eq!(
            fs::read_to_string(scope_root.join("file.txt")).unwrap(),
            "contents\n"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: 1t
    fn materialization_rejects_non_oid_tree_root_name() {
        let root = git_project("staged-snapshot-reject-tree-root-escape");
        let visible_scope = full_scope();
        let tree_materializer =
            TreeMaterializer::apply_for_tree_source(&root, TreeSource::Staged).unwrap();
        let escape = root.join("escape-root");
        let err = tree_materializer
            .materialize_visible_scope(&visible_scope, &escape.to_string_lossy())
            .unwrap_err();

        assert!(err.contains("visibleTreeOid"));
        assert!(!escape.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: 1t
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
        let tree_materializer =
            TreeMaterializer::apply_for_tree_source(&root, TreeSource::Staged).unwrap();
        let visible_tree_oid = visible_tree_oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &agent, &scope)
            .unwrap();
        let scope_root = tree_materializer
            .materialize_visible_scope(&scope, &visible_tree_oid)
            .unwrap();

        assert_read_only(&scope_root);
        assert_read_only(scope_root.join("dir"));
        assert_read_only(scope_root.join("dir/secret.txt"));

        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: 1t
    fn materialized_gitlink_is_a_read_only_leaf_file() {
        let root = git_project("staged-snapshot-gitlink-leaf-file");
        let gitlink_oid = "0123456789012345678901234567890123456789";
        let output = Command::new("git")
            .args([
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{},deps/example", gitlink_oid),
            ])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git update-index failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let agent = empty_test_agent();
        let scope = full_scope();
        let tree_materializer =
            TreeMaterializer::apply_for_tree_source(&root, TreeSource::Staged).unwrap();
        let visible_tree_oid = visible_tree_oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &agent, &scope)
            .unwrap();
        let scope_root = tree_materializer
            .materialize_visible_scope(&scope, &visible_tree_oid)
            .unwrap();
        let gitlink = scope_root.join("deps/example");

        let metadata = fs::symlink_metadata(&gitlink).unwrap();
        assert!(
            metadata.file_type().is_file(),
            "{} should be a materialized file",
            gitlink.display()
        );
        assert_eq!(
            fs::read_to_string(&gitlink).unwrap(),
            format!("gitlink {}\n", gitlink_oid)
        );
        assert_read_only(&gitlink);

        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: 1t
    fn materialized_visible_tree_uses_only_checked_git_paths_selected_by_visible_scope() {
        let root = git_project("staged-snapshot-visible-scope-pathspec-only");
        fs::create_dir_all(root.join("visible")).unwrap();
        fs::create_dir_all(root.join("hidden")).unwrap();
        fs::write(root.join("visible/keep.txt"), "keep\n").unwrap();
        fs::write(root.join("visible/untracked.txt"), "untracked\n").unwrap();
        fs::write(root.join("hidden/drop.txt"), "drop\n").unwrap();
        Command::new("git")
            .args(["add", "visible/keep.txt", "hidden/drop.txt"])
            .current_dir(&root)
            .output()
            .unwrap();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let agent = empty_test_agent();
        let visible_scope = vec!["visible".to_string()];
        let tree_materializer =
            TreeMaterializer::apply_for_tree_source(&root, TreeSource::Staged).unwrap();
        let visible_tree_oid = visible_tree_oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &agent, &visible_scope)
            .unwrap();
        let scope_root = tree_materializer
            .materialize_visible_scope(&visible_scope, &visible_tree_oid)
            .unwrap();

        assert!(scope_root.join("visible/keep.txt").is_file());
        assert!(!scope_root.join("visible/untracked.txt").exists());
        assert!(!scope_root.join("hidden").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: 1t
    fn materialized_visible_tree_keeps_file_scope_with_unrelated_exclusions() {
        let root = git_project("staged-snapshot-file-scope-with-exclusions");
        fs::create_dir_all(root.join("docker")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("docker/entrypoint"), "#!/bin/sh\n").unwrap();
        fs::write(root.join("src/hidden.rs"), "fn hidden() {}\n").unwrap();
        Command::new("git")
            .args(["add", "docker/entrypoint", "src/hidden.rs"])
            .current_dir(&root)
            .output()
            .unwrap();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let mut agent = empty_test_agent();
        agent.ignore = Some(vec!["src/**".to_string()]);
        let q_scope = vec!["docker/entrypoint".to_string()];
        let visible_scope = crate::scope::visible_scope(&agent, &q_scope).unwrap();
        let tree_materializer =
            TreeMaterializer::apply_for_tree_source(&root, TreeSource::Staged).unwrap();
        let visible_tree_oid = visible_tree_oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &agent, &q_scope)
            .unwrap();
        let scope_root = tree_materializer
            .materialize_visible_scope(&visible_scope, &visible_tree_oid)
            .unwrap();

        assert!(scope_root.join("docker/entrypoint").is_file());
        assert!(!scope_root.join("src").exists());

        let _ = fs::remove_dir_all(root);
    }
}
