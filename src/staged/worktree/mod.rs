mod extract;
mod materialize;
mod path;
mod permissions;
mod setup;
mod visible_tree;

use crate::git::{GitBlobReader, TreeSource};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;

pub(crate) struct StagedWorktreeView {
    source_root: PathBuf,
    source: TreeSource,
    materialization_root: PathBuf,
    remove_materialization_root_on_drop: bool,
    lazy_tree_dir: PathBuf,
    trees_dir: PathBuf,
    unpacked_paths: RefCell<BTreeSet<Vec<u8>>>,
    blob_reader: RefCell<Option<GitBlobReader>>,
}

impl Drop for StagedWorktreeView {
    fn drop(&mut self) {
        if self.remove_materialization_root_on_drop {
            let _ = permissions::make_materialization_tree_private(&self.trees_dir);
            let _ = std::fs::remove_dir_all(&self.materialization_root);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::config_types::AgentConfig;
    use crate::git::VisibleTreeOidCache;
    use crate::hash::full_scope;
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::path::{Path, PathBuf};
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
        let visible_scope = full_scope();
        let staged_view =
            StagedWorktreeView::apply_for_tree_source(&root, TreeSource::Staged).unwrap();
        let visible_tree_oid = visible_tree_oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &agent, &scope)
            .unwrap();
        let scope_root = staged_view
            .materialize_visible_scope(&visible_scope, &visible_tree_oid)
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
        assert_file_read_only(&gitlink);

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
