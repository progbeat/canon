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
        snapshot_root = staged_view.materialization_root().to_path_buf();
        let scope_root = staged_view
            .materialize_evaluator_scope(&empty_test_agent(), &full_scope())
            .unwrap();
        assert_ne!(scope_root, root);
        assert_eq!(
            fs::read_to_string(scope_root.join("README.md")).unwrap(),
            "staged\n"
        );
        assert!(!scope_root.join(".git").exists());
        assert!(!scope_root.join("untracked.txt").exists());
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
fn tree_worktree_view_materializes_explicit_git_tree_not_staged_changes() {
    let root = git_project("tree-snapshot-head");
    commit_all(&root, "initial");
    fs::write(root.join("README.md"), "staged\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    fs::write(root.join("README.md"), "worktree\n").unwrap();
    let source = TreeSource::resolve(&root, "HEAD", "--tree").unwrap();
    let mut visible_tree_oid_cache = VisibleTreeOidCache::new();

    let staged_view =
        StagedWorktreeView::apply_for_tree_source(&root, source, &mut visible_tree_oid_cache)
            .unwrap();
    let scope_root = staged_view
        .materialize_evaluator_scope(&empty_test_agent(), &full_scope())
        .unwrap();

    assert_eq!(
        fs::read_to_string(scope_root.join("README.md")).unwrap(),
        "hello"
    );
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "worktree\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn visible_tree_oid_for_explicit_git_tree_matches_tree_oid() {
    let root = git_project("tree-visible-oid-head");
    commit_all(&root, "initial");
    fs::write(root.join("README.md"), "staged\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    let head_tree = command_output_trimmed(
        &Command::new("git")
            .args(["rev-parse", "HEAD^{tree}"])
            .current_dir(&root)
            .output()
            .unwrap()
            .stdout,
        "git rev-parse stdout",
    )
    .unwrap()
    .to_string();
    let source = TreeSource::resolve(&root, "HEAD", "--tree").unwrap();

    let oid = VisibleTreeOidCache::new()
        .visible_tree_oid(&root, &source, &empty_test_agent(), &full_scope())
        .unwrap();

    assert_eq!(oid, head_tree);
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
        let scope_root = staged_view
            .materialize_evaluator_scope(&config.agent, &full_scope())
            .unwrap();

        assert!(!scope_root.join(".canon").exists());
        assert!(!scope_root.join("secrets/passwords.txt").exists());
        assert!(!scope_root.join("target").exists());
        assert!(scope_root.join("README.md").exists());
        assert!(!scope_root.join(".git").exists());

        let ls_files = Command::new("git")
            .args(["ls-files"])
            .current_dir(&scope_root)
            .output()
            .unwrap();
        assert!(!ls_files.status.success());

        let secret_object = Command::new("git")
            .args(["cat-file", "-e", &secret_oid])
            .current_dir(&scope_root)
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
fn staged_worktree_view_materializes_restricted_scope_without_non_scoped_files() {
    let root = git_project("staged-snapshot-restricted-scope");
    fs::create_dir_all(root.join("src/bin")).unwrap();
    fs::write(root.join("src/bin/main.rs"), "main\n").unwrap();
    fs::write(root.join("src/lib.rs"), "lib\n").unwrap();
    fs::write(root.join("README.md"), "readme\n").unwrap();
    Command::new("git")
        .args(["add", "src/bin/main.rs", "src/lib.rs", "README.md"])
        .current_dir(&root)
        .output()
        .unwrap();
    let staged_view = StagedWorktreeView::apply(&root).unwrap();

    let scope_root = staged_view
        .materialize_evaluator_scope(&empty_test_agent(), &["src/bin/main.rs".to_string()])
        .unwrap();

    assert_eq!(
        fs::read_to_string(scope_root.join("src/bin/main.rs")).unwrap(),
        "main\n"
    );
    assert!(scope_root.join("src").is_dir());
    assert!(scope_root.join("src/bin").is_dir());
    assert!(!scope_root.join("src/lib.rs").exists());
    assert!(!scope_root.join("README.md").exists());
    assert!(!scope_root.join(".git").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_materializes_git_pathspec_scope() {
    let root = git_project("staged-snapshot-pathspec-scope");
    fs::create_dir_all(root.join("src/nested")).unwrap();
    fs::write(root.join("src/lib.rs"), "lib\n").unwrap();
    fs::write(root.join("src/nested/deep.rs"), "deep\n").unwrap();
    fs::write(root.join("src/readme.txt"), "text\n").unwrap();
    Command::new("git")
        .args(["add", "src/lib.rs", "src/nested/deep.rs", "src/readme.txt"])
        .current_dir(&root)
        .output()
        .unwrap();
    let staged_view = StagedWorktreeView::apply(&root).unwrap();

    let scope_root = staged_view
        .materialize_evaluator_scope(&empty_test_agent(), &[":(glob)src/*.rs".to_string()])
        .unwrap();

    assert!(scope_root.join("src/lib.rs").exists());
    assert!(scope_root.join("src/main.rs").exists());
    assert!(!scope_root.join("src/nested/deep.rs").exists());
    assert!(!scope_root.join("src/readme.txt").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_materializes_default_pathspec_wildcards() {
    let root = git_project("staged-snapshot-default-pathspec-wildcards");
    fs::create_dir_all(root.join("src/nested")).unwrap();
    fs::write(root.join("src/lib.rs"), "lib\n").unwrap();
    fs::write(root.join("src/nested/deep.rs"), "deep\n").unwrap();
    fs::write(root.join("src/readme.txt"), "text\n").unwrap();
    Command::new("git")
        .args(["add", "src/lib.rs", "src/nested/deep.rs", "src/readme.txt"])
        .current_dir(&root)
        .output()
        .unwrap();
    let staged_view = StagedWorktreeView::apply(&root).unwrap();

    let scope_root = staged_view
        .materialize_evaluator_scope(&empty_test_agent(), &["src/*.rs".to_string()])
        .unwrap();

    assert!(scope_root.join("src/lib.rs").exists());
    assert!(scope_root.join("src/main.rs").exists());
    assert!(scope_root.join("src/nested/deep.rs").exists());
    assert!(!scope_root.join("src/readme.txt").exists());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn staged_worktree_view_restricted_scopes_hardlink_lazy_files() {
    use std::os::unix::fs::MetadataExt;

    let root = git_project("staged-snapshot-scope-hardlinks");
    fs::write(root.join("a.txt"), "GOOD\n").unwrap();
    fs::write(root.join("b.txt"), "B\n").unwrap();
    Command::new("git")
        .args(["add", "a.txt", "b.txt"])
        .current_dir(&root)
        .output()
        .unwrap();
    let staged_view = StagedWorktreeView::apply(&root).unwrap();

    let first_scope = staged_view
        .materialize_evaluator_scope(&empty_test_agent(), &["a.txt".to_string()])
        .unwrap();
    let second_scope = staged_view
        .materialize_evaluator_scope(
            &empty_test_agent(),
            &["a.txt".to_string(), "b.txt".to_string()],
        )
        .unwrap();
    let first_metadata = fs::metadata(first_scope.join("a.txt")).unwrap();
    let second_metadata = fs::metadata(second_scope.join("a.txt")).unwrap();

    assert_eq!(first_metadata.dev(), second_metadata.dev());
    assert_eq!(first_metadata.ino(), second_metadata.ino());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_reuses_restricted_scope_root_by_tree_oid() {
    let root = git_project("staged-snapshot-scope-tree-oid");
    fs::write(root.join("a.txt"), "GOOD\n").unwrap();
    Command::new("git")
        .args(["add", "a.txt"])
        .current_dir(&root)
        .output()
        .unwrap();
    let staged_view = StagedWorktreeView::apply(&root).unwrap();

    let first_scope = staged_view
        .materialize_evaluator_scope(&empty_test_agent(), &["a.txt".to_string()])
        .unwrap();
    let second_scope = staged_view
        .materialize_evaluator_scope(&empty_test_agent(), &["a.txt".to_string()])
        .unwrap();

    assert_eq!(first_scope, second_scope);
    assert_eq!(
        first_scope.parent().unwrap().file_name().unwrap().to_str(),
        Some("scopes")
    );
    assert_eq!(
        fs::read_to_string(first_scope.join("a.txt")).unwrap(),
        "GOOD\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_full_scope_returns_scope_root_by_tree_oid() {
    let root = git_project("staged-snapshot-full-scope-root");
    fs::write(root.join("a.txt"), "GOOD\n").unwrap();
    Command::new("git")
        .args(["add", "a.txt"])
        .current_dir(&root)
        .output()
        .unwrap();
    let staged_view = StagedWorktreeView::apply(&root).unwrap();

    let full = staged_view
        .materialize_evaluator_scope(&empty_test_agent(), &full_scope())
        .unwrap();

    assert_ne!(full, staged_view.materialization_root().join("lazy"));
    assert_eq!(
        full.parent().unwrap().file_name().unwrap().to_str(),
        Some("scopes")
    );
    assert_eq!(fs::read_to_string(full.join("a.txt")).unwrap(), "GOOD\n");
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn staged_worktree_view_scope_directories_are_read_only() {
    let root = git_project("staged-snapshot-private-dirs");
    fs::create_dir_all(root.join("dir")).unwrap();
    fs::write(root.join("dir/secret.txt"), "secret\n").unwrap();
    Command::new("git")
        .args(["add", "dir/secret.txt"])
        .current_dir(&root)
        .output()
        .unwrap();
    let staged_view = StagedWorktreeView::apply(&root).unwrap();
    let materialization_root = staged_view.materialization_root().to_path_buf();
    let scope_root = staged_view
        .materialize_evaluator_scope(&empty_test_agent(), &full_scope())
        .unwrap();

    assert_private_dir(&materialization_root);
    assert_private_dir(&materialization_root.join("lazy"));
    assert_private_dir(&materialization_root.join("lazy/dir"));
    assert_private_dir(&materialization_root.join("scopes"));
    assert_read_only_dir(&scope_root);
    assert_read_only_dir(&scope_root.join("dir"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_excludes_gitlinks_from_blob_file_entries() {
    let root = git_project("staged-snapshot-gitlink");
    let output = Command::new("git")
        .args([
            "update-index",
            "--add",
            "--cacheinfo",
            "160000,1111111111111111111111111111111111111111,vendor/submodule",
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let staged_view = StagedWorktreeView::apply(&root).unwrap();
    let scope_root = staged_view
        .materialize_evaluator_scope(&empty_test_agent(), &full_scope())
        .unwrap();

    assert!(scope_root.join("README.md").exists());
    assert!(!scope_root.join("vendor/submodule").exists());
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
        let scope_root = staged_view
            .materialize_evaluator_scope(&config.agent, &full_scope())
            .unwrap();

        assert!(!scope_root.join(".git").exists());
        assert_eq!(
            fs::read_to_string(scope_root.join("README.md")).unwrap(),
            "staged\n"
        );
        let ls_files = Command::new("git")
            .args(["ls-files"])
            .current_dir(&scope_root)
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
        let scope_root = staged_view
            .materialize_evaluator_scope(&config.agent, &full_scope())
            .unwrap();

        assert!(!scope_root.join(".git").exists());
        let ls_files = Command::new("git")
            .args(["ls-files"])
            .current_dir(&scope_root)
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
        let scope_root = staged_view
            .materialize_evaluator_scope(&config.agent, &full_scope())
            .unwrap();

        assert!(!scope_root.join(".git").exists());
        let ls_files = Command::new("git")
            .args(["ls-files"])
            .current_dir(&scope_root)
            .output()
            .unwrap();
        assert!(!ls_files.status.success());
        assert!(scope_root.join("README.md").exists());
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
        let scope_root = staged_view
            .materialize_evaluator_scope(&empty_test_agent(), &full_scope())
            .unwrap();
        assert_eq!(
            fs::read_to_string(scope_root.join("README.md")).unwrap(),
            "staged\n"
        );

        assert!(scope_root.join("README.md").exists());
        assert!(scope_root.join("ADDED.md").exists());
        #[cfg(unix)]
        assert!(scope_root.join(literal_added).exists());
        assert!(scope_root.join("new-name.txt").exists());
        assert!(!scope_root.join("old-name.txt").exists());
        assert!(!scope_root.join("src/main.rs").exists());

        let log = Command::new("git")
            .args(["log", "--oneline", "-1"])
            .current_dir(&scope_root)
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
    let hook_path = resolve_git_path(&root, PRE_COMMIT_HOOK_PATH).unwrap();
    let expected_hooks_path = resolve_git_path(&root, GIT_HOOKS_PATH).unwrap();
    fs::create_dir_all(hook_path.parent().unwrap()).unwrap();
    fs::write(&hook_path, "local hook content\n").unwrap();
    Command::new("git")
        .args([
            "config",
            "--local",
            "core.hooksPath",
            expected_hooks_path.to_str().unwrap(),
        ])
        .current_dir(&root)
        .output()
        .unwrap();

    {
        let staged_view = StagedWorktreeView::apply(&root).unwrap();
        let scope_root = staged_view
            .materialize_evaluator_scope(&empty_test_agent(), &full_scope())
            .unwrap();
        let hooks_path = Command::new("git")
            .args(["config", "--local", "--get", "core.hooksPath"])
            .current_dir(&scope_root)
            .output()
            .unwrap();
        assert!(!hooks_path.status.success());
        assert!(String::from_utf8_lossy(&hooks_path.stdout)
            .trim()
            .is_empty());
        assert!(!scope_root.join(PRE_COMMIT_HOOK_PATH).exists());
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_visible_tree_oid_ignores_local_git_hook_metadata() {
    let root = git_project("visible-tree-oid-hooks");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let scope = vec![".git".to_string()];
    let before = staged_visible_tree_oid(&root, &config.agent, &scope).unwrap();

    let hook_path = resolve_git_path(&root, PRE_COMMIT_HOOK_PATH).unwrap();
    let hooks_path = resolve_git_path(&root, GIT_HOOKS_PATH).unwrap();
    fs::create_dir_all(hook_path.parent().unwrap()).unwrap();
    fs::write(&hook_path, DEFAULT_PRE_COMMIT_HOOK).unwrap();
    Command::new("git")
        .args([
            "config",
            "--local",
            "core.hooksPath",
            hooks_path.to_str().unwrap(),
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    let after_install = staged_visible_tree_oid(&root, &config.agent, &scope).unwrap();

    fs::write(&hook_path, "changed\n").unwrap();
    let after_hook_change = staged_visible_tree_oid(&root, &config.agent, &scope).unwrap();

    assert_eq!(before, after_install);
    assert_eq!(after_install, after_hook_change);
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn read_git_blobs_reaps_git_cat_file_after_stdin_write_failure() {
    use std::os::unix::fs::PermissionsExt;

    let fake_git_dir = TestDir::new("fake-git-closes-stdin");
    let fake_git = fake_git_dir.path().join("git");
    fs::write(&fake_git, "#!/bin/sh\nexec 0<&-\nsleep 0.05\nexit 1\n").unwrap();
    fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o755)).unwrap();
    let object_ids = vec!["0123456789012345678901234567890123456789".to_string(); 8192];

    let err = read_git_blobs_with_git_program(Path::new("/"), &object_ids, fake_git.as_os_str())
        .unwrap_err();

    assert!(err.contains("failed to write git cat-file input"), "{err}");
}

#[cfg(unix)]
#[test]
fn git_blob_reader_reuses_one_cat_file_process_across_reads() {
    use std::os::unix::fs::PermissionsExt;

    let fake_git_dir = TestDir::new("fake-git-batch-reader");
    let fake_git = fake_git_dir.path().join("git");
    let spawn_count = fake_git_dir.path().join("spawn-count");
    fs::write(
        &fake_git,
        "#!/bin/sh\ncounter=\"$(dirname \"$0\")/spawn-count\"\necho spawn >> \"$counter\"\nwhile IFS= read -r oid; do\n  printf '%s blob 1\\nx\\n' \"$oid\"\ndone\n",
    )
    .unwrap();
    fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o755)).unwrap();
    let mut reader =
        GitBlobReader::new_with_test_git_program(Path::new("/"), fake_git.as_os_str()).unwrap();

    let first = reader
        .read_blobs(&["0123456789012345678901234567890123456789".to_string()])
        .unwrap();
    let second = reader
        .read_blobs(&["abcdefabcdefabcdefabcdefabcdefabcdefabcd".to_string()])
        .unwrap();
    drop(reader);

    assert_eq!(first, vec![b"x".to_vec()]);
    assert_eq!(second, vec![b"x".to_vec()]);
    assert_eq!(fs::read_to_string(spawn_count).unwrap().lines().count(), 1);
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

#[cfg(unix)]
#[test]
fn staged_snapshot_parent_candidates_prefer_common_memory_backed_locations() {
    let candidates = crate::platform::staged_snapshot_parent_candidates();

    assert_eq!(candidates[0], PathBuf::from("/dev/shm"));
    assert!(candidates.contains(&PathBuf::from("/run/shm")));
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
        let scope_root = staged_view
            .materialize_evaluator_scope(&empty_test_agent(), &full_scope())
            .unwrap();
        assert_eq!(
            fs::read_to_string(scope_root.join("README.md")).unwrap(),
            "staged\n"
        );
        assert!(!scope_root.join("ignored.txt").exists());
    }

    assert_eq!(
        fs::read_to_string(root.join("ignored.txt")).unwrap(),
        "ignored\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn visible_tree_oid_cache_reuses_staged_entries_between_count_and_hash() {
    let root = git_project("visible-tree-cache-entries");
    let agent = empty_test_agent();
    let mut cache = VisibleTreeOidCache::new();

    let file_count = cache
        .staged_visible_file_count(&root, &agent, &full_scope())
        .unwrap();

    assert_eq!(file_count, 2);
    assert_eq!(cache.staged_entries_cache_len(), 1);

    let _oid = cache
        .staged_visible_tree_oid(&root, &agent, &full_scope())
        .unwrap();

    assert_eq!(cache.staged_entries_cache_len(), 1);
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
        let scope_root = staged_view
            .materialize_evaluator_scope(&empty_test_agent(), &full_scope())
            .unwrap();
        assert_eq!(
            fs::read_to_string(scope_root.join(special)).unwrap(),
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
fn staged_worktree_view_materializes_symlinks_as_symlinks() {
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
        let scope_root = staged_view
            .materialize_evaluator_scope(&empty_test_agent(), &full_scope())
            .unwrap();
        let snapshot_link = scope_root.join("outside-link");
        let metadata = fs::symlink_metadata(&snapshot_link).unwrap();
        assert!(metadata.file_type().is_symlink());
        assert_eq!(
            fs::read_link(snapshot_link).unwrap(),
            std::path::PathBuf::from("/tmp/canon-outside-target")
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

fn empty_test_agent() -> AgentConfig {
    AgentConfig {
        models: Vec::new(),
        thinking: "medium".to_string(),
        instructions: None,
        ignore: Vec::new(),
        plugins: Vec::new(),
    }
}

#[cfg(unix)]
fn assert_private_dir(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "{} mode is {:o}", path.display(), mode);
}

#[cfg(unix)]
fn assert_read_only_dir(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o555, "{} mode is {:o}", path.display(), mode);
}

#[cfg(unix)]
#[test]
fn git_stdout_path_preserves_non_utf8_bytes() {
    use std::os::unix::ffi::OsStrExt;

    let path = path_from_git_stdout(vec![b'/', b't', 0xff, b'\n']).unwrap();

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
fn staged_worktree_view_materializes_non_utf8_paths() {
    let root = git_project("staged-snapshot-non-utf8-path");
    let name = git_path_from_raw_bytes(b"nonutf8-\xff.txt").unwrap();
    fs::write(root.join(&name), "staged\n").unwrap();
    let output = Command::new("git")
        .arg("add")
        .arg("--")
        .arg(&name)
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let staged_view = StagedWorktreeView::apply(&root).unwrap();
    let scope_root = staged_view
        .materialize_evaluator_scope(&empty_test_agent(), &full_scope())
        .unwrap();

    assert_eq!(
        fs::read_to_string(scope_root.join(name)).unwrap(),
        "staged\n"
    );
    let _ = fs::remove_dir_all(root);
}
