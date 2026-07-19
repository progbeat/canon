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
    tmp_dir: PathBuf,
    canon_owns_tmp_dir: bool,
    invocation_journal: Option<RefCell<InvocationMaterializationJournal>>,
    lazy_tree_dir: PathBuf,
    trees_dir: PathBuf,
    unpacked_paths: RefCell<BTreeSet<Vec<u8>>>,
    blob_reader: RefCell<Option<GitBlobReader>>,
}

struct InvocationMaterializationJournal {
    lazy_tree_dir_created: bool,
    trees_dir_created: bool,
    lazy_path_changes: Vec<InvocationLazyPathChange>,
    created_visible_tree_roots: BTreeSet<PathBuf>,
}

struct InvocationLazyPathChange {
    target: PathBuf,
    backup: Option<PathBuf>,
}

impl Drop for StagedWorktreeView {
    fn drop(&mut self) {
        if self.canon_owns_tmp_dir {
            let _ = permissions::make_materialization_tree_private(&self.trees_dir);
            let _ = std::fs::remove_dir_all(&self.tmp_dir);
            return;
        }
        let Some(journal) = self.invocation_journal.as_mut() else {
            return;
        };
        let journal = journal.get_mut();
        for tree_root in &journal.created_visible_tree_roots {
            let _ = remove_materialization_path(tree_root);
        }
        for change in journal.lazy_path_changes.iter().rev() {
            let _ = remove_materialization_path(&change.target);
            if let Some(backup) = &change.backup {
                let _ = std::fs::rename(backup, &change.target);
            }
            remove_empty_lazy_parent_dirs(&change.target, &self.lazy_tree_dir);
        }
        if journal.trees_dir_created {
            let _ = std::fs::remove_dir(&self.trees_dir);
        }
        if journal.lazy_tree_dir_created {
            let _ = std::fs::remove_dir(&self.lazy_tree_dir);
        }
    }
}

fn remove_materialization_path(path: &std::path::Path) -> Result<(), std::io::Error> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    if metadata.file_type().is_dir() {
        permissions::make_materialization_tree_private(path).map_err(std::io::Error::other)?;
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

fn remove_empty_lazy_parent_dirs(path: &std::path::Path, lazy_tree_dir: &std::path::Path) {
    let mut parent = path.parent();
    while let Some(directory) = parent {
        if directory == lazy_tree_dir {
            break;
        }
        if std::fs::remove_dir(directory).is_err() {
            break;
        }
        parent = directory.parent();
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

    #[test] // xpec: ig
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

    #[test] // xpec: ig
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

    #[test] // xpec: ig
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

    #[test] // xpec: ig
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
        let staged_view =
            StagedWorktreeView::apply_for_tree_source(&root, TreeSource::Staged).unwrap();
        let visible_tree_oid = visible_tree_oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &agent, &visible_scope)
            .unwrap();
        let scope_root = staged_view
            .materialize_visible_scope(&visible_scope, &visible_tree_oid)
            .unwrap();

        assert!(scope_root.join("visible/keep.txt").is_file());
        assert!(!scope_root.join("visible/untracked.txt").exists());
        assert!(!scope_root.join("hidden").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: ig
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
        let staged_view =
            StagedWorktreeView::apply_for_tree_source(&root, TreeSource::Staged).unwrap();
        let visible_tree_oid = visible_tree_oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &agent, &q_scope)
            .unwrap();
        let scope_root = staged_view
            .materialize_visible_scope(&visible_scope, &visible_tree_oid)
            .unwrap();

        assert!(scope_root.join("docker/entrypoint").is_file());
        assert!(!scope_root.join("src").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: ig
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

    #[test] // xpec: ig,Ky,M
    fn invocation_local_materialization_is_removed_on_drop() {
        let root = git_project("invocation-local-materialization-cleanup");
        let staged_view =
            StagedWorktreeView::apply_invocation_local_for_tree_source(&root, TreeSource::Staged)
                .unwrap();
        let tmp_dir = staged_view.tmp_dir.clone();

        assert!(staged_view.canon_owns_tmp_dir);
        assert!(tmp_dir.is_dir());
        drop(staged_view);
        assert!(!tmp_dir.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: ig,Ky
    fn invocation_local_materialization_restores_existing_configured_cache() {
        let root = git_project("invocation-local-configured-cache-restore");
        fs::write(root.join("file.txt"), "new staged contents\n").unwrap();
        Command::new("git")
            .args(["add", "file.txt"])
            .current_dir(&root)
            .output()
            .unwrap();
        let cache = root.join("tree-cache");
        fs::create_dir_all(cache.join("lazy")).unwrap();
        fs::create_dir_all(cache.join("trees")).unwrap();
        fs::write(cache.join("lazy/file.txt"), "preexisting cache contents\n").unwrap();
        fs::write(cache.join("trees/preexisting"), "keep\n").unwrap();
        let staged_view = StagedWorktreeView::apply_invocation_local_for_tree_source_at(
            &root,
            TreeSource::Staged,
            &cache,
        )
        .unwrap();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let agent = empty_test_agent();
        let scope = full_scope();
        let visible_tree_oid = visible_tree_oid_cache
            .visible_tree_oid(&root, &TreeSource::Staged, &agent, &scope)
            .unwrap();

        let scope_root = staged_view
            .materialize_visible_scope(&scope, &visible_tree_oid)
            .unwrap();

        assert_eq!(staged_view.tmp_dir, cache);
        assert_eq!(
            fs::read_to_string(cache.join("lazy/file.txt")).unwrap(),
            "new staged contents\n"
        );
        assert_eq!(
            fs::read_to_string(scope_root.join("file.txt")).unwrap(),
            "new staged contents\n"
        );
        drop(staged_view);
        assert_eq!(
            fs::read_to_string(cache.join("lazy/file.txt")).unwrap(),
            "preexisting cache contents\n"
        );
        assert!(!scope_root.exists());
        assert_eq!(
            fs::read_to_string(cache.join("trees/preexisting")).unwrap(),
            "keep\n"
        );

        let _ = fs::remove_dir_all(root);
    }

    fn empty_test_agent() -> AgentConfig {
        AgentConfig {
            models: Vec::new(),
            thinking: "medium".to_string(),
            ignore: None,
            ignore_configured: false,
            plugins: Vec::new(),
        }
    }

    fn assert_dir_read_only(path: &Path) {
        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        // xpec: ig
        assert_ne!(mode & 0o555, 0, "{} should be readable", path.display());
        // xpec: ig
        assert_eq!(mode & 0o222, 0, "{} should not be writable", path.display());
    }

    fn assert_file_read_only(path: &Path) {
        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        // xpec: ig
        assert_ne!(mode & 0o444, 0, "{} should be readable", path.display());
        // xpec: ig
        assert_eq!(mode & 0o222, 0, "{} should not be writable", path.display());
    }

    fn assert_symlink_target(path: &Path, expected: &Path) {
        let metadata = fs::symlink_metadata(path).unwrap();
        // xpec: ig
        assert!(
            metadata.file_type().is_symlink(),
            "{} should be a symlink",
            path.display()
        );
        // xpec: ig
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
            // xpec: ig
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
