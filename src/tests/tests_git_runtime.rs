use super::*;

#[test]
fn git_project_root_finds_top_level_from_subdirectory() {
    let root = git_project("git-root-subdir");
    let subdir = root.join(".canon");
    fs::create_dir_all(&subdir).unwrap();
    assert_eq!(
        fs::canonicalize(git_project_root(&subdir).unwrap()).unwrap(),
        fs::canonicalize(&root).unwrap()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_materializes_staged_snapshot_without_touching_worktree() {
    let root = git_project("staged-snapshot-worktree");
    commit_all(&root, "initial");
    fs::write(root.join("README.md"), "staged\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    fs::write(root.join("README.md"), "unstaged\n").unwrap();
    fs::write(root.join("untracked.txt"), "untracked\n").unwrap();
    let stash_count_before = stash_count(&root);
    let snapshot_root;

    {
        let staged_view = StagedWorktreeView::apply(&root).unwrap();
        snapshot_root = staged_view.snapshot_root().to_path_buf();
        assert_ne!(snapshot_root, root);
        assert_eq!(
            fs::read_to_string(snapshot_root.join("README.md")).unwrap(),
            "staged\n"
        );
        assert!(snapshot_root.join(".git").exists());
        assert!(!snapshot_root.join("untracked.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("README.md")).unwrap(),
            "unstaged\n"
        );
        assert!(root.join("untracked.txt").exists());
        assert_eq!(stash_count(&root), stash_count_before);
    }

    assert!(!snapshot_root.exists());
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "unstaged\n"
    );
    assert!(root.join("untracked.txt").exists());
    let diff = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&diff.stdout).trim(), "README.md");
    assert_eq!(stash_count(&root), stash_count_before);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_removes_evaluator_denied_paths_from_snapshot() {
    let root = git_project("staged-snapshot-evaluator-deny");
    write_check_config(&root);
    fs::create_dir_all(root.join(".canon/draft")).unwrap();
    fs::write(root.join(".canon/draft/private.md"), "draft\n").unwrap();
    fs::create_dir_all(root.join("target")).unwrap();
    fs::write(root.join("target/cache.txt"), "cache\n").unwrap();
    fs::create_dir_all(root.join("secrets")).unwrap();
    fs::write(root.join("secrets/passwords.txt"), "secret\n").unwrap();
    Command::new("git")
        .args([
            "add",
            ".canon/check.yml",
            ".canon/draft/private.md",
            "secrets/passwords.txt",
            "target/cache.txt",
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    let secret_oid_output = Command::new("git")
        .args(["hash-object", "secrets/passwords.txt"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(secret_oid_output.status.success());
    let secret_oid = String::from_utf8(secret_oid_output.stdout)
        .unwrap()
        .trim()
        .to_string();
    let mut config = parse_check_config(check_config_yaml()).unwrap();
    config.agent.ignore.push("secrets/*".to_string());

    {
        let staged_view = StagedWorktreeView::apply(&root).unwrap();
        assert!(staged_view
            .snapshot_root()
            .join(".canon/draft/private.md")
            .exists());
        assert!(staged_view
            .snapshot_root()
            .join("target/cache.txt")
            .exists());
        staged_view
            .remove_evaluator_denied_paths(&config.agent)
            .unwrap();

        assert!(!staged_view.snapshot_root().join(".canon").exists());
        assert!(!staged_view
            .snapshot_root()
            .join("secrets/passwords.txt")
            .exists());
        assert!(!staged_view.snapshot_root().join("target").exists());
        assert!(staged_view.snapshot_root().join("README.md").exists());
        assert!(staged_view.snapshot_root().join(".git").exists());

        let ls_files = Command::new("git")
            .args(["ls-files"])
            .current_dir(staged_view.snapshot_root())
            .output()
            .unwrap();
        assert!(
            ls_files.status.success(),
            "{}",
            String::from_utf8_lossy(&ls_files.stderr)
        );
        let files = String::from_utf8_lossy(&ls_files.stdout);
        assert!(!files.contains(".canon/"));
        assert!(!files.contains("secrets/passwords.txt"));
        assert!(!files.contains("target/cache.txt"));

        let secret_object = Command::new("git")
            .args(["cat-file", "-e", &secret_oid])
            .current_dir(staged_view.snapshot_root())
            .output()
            .unwrap();
        assert!(!secret_object.status.success());
    }
    assert!(root.join(".canon/draft/private.md").exists());
    assert!(root.join("secrets/passwords.txt").exists());
    assert!(root.join("target/cache.txt").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_removes_git_metadata_when_git_is_denied() {
    let root = git_project("staged-snapshot-deny-git");
    fs::write(root.join("README.md"), "staged\n").unwrap();
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(&root)
        .output()
        .unwrap();
    let mut config = parse_check_config(check_config_yaml()).unwrap();
    config.agent.ignore.push(".git".to_string());

    {
        let staged_view = StagedWorktreeView::apply(&root).unwrap();
        staged_view
            .remove_evaluator_denied_paths(&config.agent)
            .unwrap();

        assert!(!staged_view.snapshot_root().join(".git").exists());
        assert_eq!(
            fs::read_to_string(staged_view.snapshot_root().join("README.md")).unwrap(),
            "staged\n"
        );
        let ls_files = Command::new("git")
            .args(["ls-files"])
            .current_dir(staged_view.snapshot_root())
            .output()
            .unwrap();
        assert!(!ls_files.status.success());
    }

    assert!(root.join(".git").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_removes_git_metadata_when_git_tree_is_denied() {
    let root = git_project("staged-snapshot-deny-git-tree");
    fs::write(root.join("README.md"), "staged\n").unwrap();
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(&root)
        .output()
        .unwrap();
    let mut config = parse_check_config(check_config_yaml()).unwrap();
    config.agent.ignore.push(".git/**".to_string());

    {
        let staged_view = StagedWorktreeView::apply(&root).unwrap();
        staged_view
            .remove_evaluator_denied_paths(&config.agent)
            .unwrap();

        assert!(!staged_view.snapshot_root().join(".git").exists());
        let ls_files = Command::new("git")
            .args(["ls-files"])
            .current_dir(staged_view.snapshot_root())
            .output()
            .unwrap();
        assert!(!ls_files.status.success());
    }

    assert!(root.join(".git").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_removes_denied_rebuilt_git_metadata_paths() {
    let root = git_project("staged-snapshot-deny-git-config");
    fs::write(root.join("README.md"), "staged\n").unwrap();
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(&root)
        .output()
        .unwrap();
    let mut config = parse_check_config(check_config_yaml()).unwrap();
    config.agent.ignore.push(".git/config".to_string());

    {
        let staged_view = StagedWorktreeView::apply(&root).unwrap();
        staged_view
            .remove_evaluator_denied_paths(&config.agent)
            .unwrap();

        assert!(staged_view.snapshot_root().join(".git").exists());
        assert!(!staged_view.snapshot_root().join(".git/config").exists());
        let ls_files = Command::new("git")
            .args(["ls-files"])
            .current_dir(staged_view.snapshot_root())
            .output()
            .unwrap();
        assert!(
            ls_files.status.success(),
            "{}",
            String::from_utf8_lossy(&ls_files.stderr)
        );
        let files = String::from_utf8_lossy(&ls_files.stdout);
        assert!(files.lines().any(|path| path == "README.md"));
        assert!(!files.lines().any(|path| path == ".git/config"));
    }

    assert!(root.join(".git/config").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_exposes_staged_index_without_git_history() {
    let root = git_project("staged-snapshot-git-commands");
    fs::write(root.join("old-name.txt"), "renamed\n").unwrap();
    Command::new("git")
        .args(["add", "old-name.txt"])
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "initial");
    fs::write(root.join("README.md"), "staged\n").unwrap();
    fs::write(root.join("ADDED.md"), "added\n").unwrap();
    #[cfg(unix)]
    let literal_added = ":(literal)added.md";
    #[cfg(unix)]
    fs::write(root.join(literal_added), "literal added\n").unwrap();
    fs::remove_file(root.join("src/main.rs")).unwrap();
    Command::new("git")
        .args(["mv", "old-name.txt", "new-name.txt"])
        .current_dir(&root)
        .output()
        .unwrap();
    #[cfg(unix)]
    let mut add_args = vec!["add", "--", "README.md", "ADDED.md", "src/main.rs"];
    #[cfg(not(unix))]
    let add_args = vec!["add", "--", "README.md", "ADDED.md", "src/main.rs"];
    #[cfg(unix)]
    add_args.insert(4, literal_added);
    Command::new("git")
        .arg("--literal-pathspecs")
        .args(add_args)
        .current_dir(&root)
        .output()
        .unwrap();
    fs::write(root.join("README.md"), "unstaged\n").unwrap();

    {
        let staged_view = StagedWorktreeView::apply(&root).unwrap();
        assert_eq!(
            fs::read_to_string(staged_view.snapshot_root().join("README.md")).unwrap(),
            "staged\n"
        );

        let ls_files = Command::new("git")
            .args(["ls-files"])
            .current_dir(staged_view.snapshot_root())
            .output()
            .unwrap();
        assert!(
            ls_files.status.success(),
            "{}",
            String::from_utf8_lossy(&ls_files.stderr)
        );
        let files = String::from_utf8_lossy(&ls_files.stdout)
            .lines()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        assert!(files.iter().any(|path| path == "README.md"));
        assert!(files.iter().any(|path| path == "ADDED.md"));
        #[cfg(unix)]
        assert!(files.iter().any(|path| path == literal_added));
        assert!(files.iter().any(|path| path == "new-name.txt"));
        assert!(!files.iter().any(|path| path == "old-name.txt"));
        assert!(!files.iter().any(|path| path == "src/main.rs"));

        let log = Command::new("git")
            .args(["log", "--oneline", "-1"])
            .current_dir(staged_view.snapshot_root())
            .output()
            .unwrap();
        assert!(!log.status.success());
    }

    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "unstaged\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_excludes_local_hook_config_and_hook_file() {
    let root = git_project("staged-snapshot-hooks");
    commit_all(&root, "initial");
    fs::create_dir_all(root.join(PRE_COMMIT_HOOK_PATH).parent().unwrap()).unwrap();
    fs::write(root.join(PRE_COMMIT_HOOK_PATH), "local hook content\n").unwrap();
    Command::new("git")
        .args(["config", "--local", "core.hooksPath", GIT_HOOKS_PATH])
        .current_dir(&root)
        .output()
        .unwrap();

    {
        let staged_view = StagedWorktreeView::apply(&root).unwrap();
        let hooks_path = Command::new("git")
            .args(["config", "--local", "--get", "core.hooksPath"])
            .current_dir(staged_view.snapshot_root())
            .output()
            .unwrap();
        assert!(!hooks_path.status.success());
        assert!(String::from_utf8_lossy(&hooks_path.stdout)
            .trim()
            .is_empty());
        assert!(!staged_view
            .snapshot_root()
            .join(PRE_COMMIT_HOOK_PATH)
            .exists());
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_scope_hash_ignores_local_git_hook_metadata() {
    let root = git_project("scope-hash-hooks");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let scope = vec![".git".to_string()];
    let before = staged_scope_hash(&root, &config.agent, &scope).unwrap();

    fs::create_dir_all(root.join(PRE_COMMIT_HOOK_PATH).parent().unwrap()).unwrap();
    fs::write(root.join(PRE_COMMIT_HOOK_PATH), DEFAULT_PRE_COMMIT_HOOK).unwrap();
    Command::new("git")
        .args(["config", "--local", "core.hooksPath", GIT_HOOKS_PATH])
        .current_dir(&root)
        .output()
        .unwrap();
    let after_install = staged_scope_hash(&root, &config.agent, &scope).unwrap();

    fs::write(root.join(PRE_COMMIT_HOOK_PATH), "changed\n").unwrap();
    let after_hook_change = staged_scope_hash(&root, &config.agent, &scope).unwrap();

    assert_eq!(before, after_install);
    assert_eq!(after_install, after_hook_change);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_snapshot_parent_must_be_outside_project_root() {
    let root = git_project("staged-snapshot-parent-outside");
    let root = fs::canonicalize(root).unwrap();
    let inside = root.join("tmp");
    fs::create_dir_all(&inside).unwrap();
    assert!(snapshot_parent_outside_worktree(&root, &root).is_err());
    assert!(snapshot_parent_outside_worktree(&root, &inside).is_err());
    assert!(snapshot_parent_outside_worktree(&root, root.parent().unwrap()).is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_leaves_ignored_worktree_files_outside_snapshot() {
    let root = git_project("staged-snapshot-ignored");
    fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg(".gitignore")
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "ignore file");
    fs::write(root.join("README.md"), "staged\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    fs::write(root.join("ignored.txt"), "ignored\n").unwrap();

    {
        let staged_view = StagedWorktreeView::apply(&root).unwrap();
        assert_eq!(
            fs::read_to_string(staged_view.snapshot_root().join("README.md")).unwrap(),
            "staged\n"
        );
        assert!(!staged_view.snapshot_root().join("ignored.txt").exists());
    }

    assert_eq!(
        fs::read_to_string(root.join("ignored.txt")).unwrap(),
        "ignored\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
#[cfg(unix)]
fn staged_worktree_view_materializes_literal_pathspec_names_from_index() {
    let root = git_project("staged-snapshot-literal-pathspec");
    commit_all(&root, "initial");
    let special = ":(literal)name.txt";
    fs::write(root.join(special), "staged\n").unwrap();
    let output = Command::new("git")
        .args(["--literal-pathspecs", "add", "--", special])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(root.join(special), "unstaged\n").unwrap();

    {
        let staged_view = StagedWorktreeView::apply(&root).unwrap();
        assert_eq!(
            fs::read_to_string(staged_view.snapshot_root().join(special)).unwrap(),
            "staged\n"
        );
    }

    assert_eq!(
        fs::read_to_string(root.join(special)).unwrap(),
        "unstaged\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn staged_worktree_view_materializes_symlinks_as_regular_files() {
    use std::os::unix::fs::symlink;

    let root = git_project("staged-snapshot-symlink");
    symlink("/tmp/canon-outside-target", root.join("outside-link")).unwrap();
    let output = Command::new("git")
        .args(["add", "outside-link"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    {
        let staged_view = StagedWorktreeView::apply(&root).unwrap();
        let snapshot_link = staged_view.snapshot_root().join("outside-link");
        let metadata = fs::symlink_metadata(&snapshot_link).unwrap();
        assert!(!metadata.file_type().is_symlink());
        assert!(metadata.file_type().is_file());
        assert_eq!(
            fs::read_to_string(snapshot_link).unwrap(),
            "/tmp/canon-outside-target"
        );
    }

    assert!(fs::symlink_metadata(root.join("outside-link"))
        .unwrap()
        .file_type()
        .is_symlink());
    let _ = fs::remove_dir_all(root);
}

fn stash_count(root: &Path) -> usize {
    let output = Command::new("git")
        .args(["stash", "list", "--format=%H"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().lines().count()
}

#[cfg(unix)]
#[test]
fn git_stdout_path_preserves_non_utf8_bytes() {
    use std::os::unix::ffi::OsStrExt;

    let path = path_from_git_stdout(vec![b'/', b't', 0xff, b'\n']);

    assert_eq!(path.as_os_str().as_bytes(), &[b'/', b't', 0xff]);
}

#[cfg(unix)]
#[test]
fn checkout_index_prefix_preserves_non_utf8_snapshot_path() {
    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let path = PathBuf::from(OsString::from_vec(b"/tmp/canon-\xff".to_vec()));
    let arg = crate::platform::checkout_index_prefix_arg(&path).unwrap();

    assert_eq!(arg.as_os_str().as_bytes(), b"--prefix=/tmp/canon-\xff/");
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn initialize_snapshot_git_repo_accepts_non_utf8_snapshot_path() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let parent = TestDir::new("staged-snapshot-non-utf8-init");
    let snapshot_root = parent
        .path()
        .join(OsString::from_vec(b"snapshot-\xff".to_vec()));
    fs::create_dir(&snapshot_root).unwrap();

    initialize_snapshot_git_repo_for_test(&snapshot_root).unwrap();

    let ls_files = Command::new("git")
        .args(["ls-files"])
        .current_dir(&snapshot_root)
        .output()
        .unwrap();
    assert!(
        ls_files.status.success(),
        "{}",
        String::from_utf8_lossy(&ls_files.stderr)
    );
}
