use super::*;

#[test]
fn hook_install_creates_reusable_pre_commit_hook() {
    let root = git_project("hook-install");
    run_hook_install(&root).unwrap();
    let hook_path = managed_pre_commit_hook_path(&root);
    assert!(!root.join(CHECK_PATH).exists());
    assert!(!root.join(".gitignore").exists());
    assert_eq!(DEFAULT_PRE_COMMIT_HOOK, "#!/usr/bin/env sh\ncanon gate\n");
    assert!(!DEFAULT_PRE_COMMIT_HOOK.contains("git status --porcelain -- .canon/"));
    assert_eq!(
        fs::read_to_string(&hook_path).unwrap(),
        DEFAULT_PRE_COMMIT_HOOK
    );
    assert_eq!(
        DEFAULT_PRE_COMMIT_HOOK.matches("canon gate failed").count(),
        0
    );
    assert!(!DEFAULT_PRE_COMMIT_HOOK.contains("target/debug/canon"));
    assert!(!DEFAULT_PRE_COMMIT_HOOK.contains(".codex-plugin"));
    assert!(!DEFAULT_PRE_COMMIT_HOOK.contains("run canon check before committing"));
    assert_hooks_path_resolves_to_managed(&root);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(
            fs::metadata(&hook_path).unwrap().permissions().mode() & 0o111,
            0
        );
    }

    run_hook_install(&root).unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_install_uses_relative_hooks_path_that_survives_repo_move() {
    let root = git_project("hook-install-repo-move");
    run_hook_install(&root).unwrap();
    assert_eq!(
        PathBuf::from(current_git_hooks_path_for_worktree(&root).unwrap().unwrap()),
        PathBuf::from(".git/canon/hooks")
    );
    let moved = temp_home("hook-install-repo-moved");
    fs::rename(&root, &moved).unwrap();

    run_hook_install(&moved).unwrap();

    assert_hooks_path_resolves_to_managed(&moved);
    assert_eq!(
        fs::read_to_string(managed_pre_commit_hook_path(&moved)).unwrap(),
        DEFAULT_PRE_COMMIT_HOOK
    );
    let _ = fs::remove_dir_all(moved);
}

#[test]
fn hook_install_refuses_non_exact_existing_canon_pre_commit_hook() {
    let root = git_project("hook-install-update");
    let hook_path = managed_pre_commit_hook_path(&root);
    fs::create_dir_all(hook_path.parent().unwrap()).unwrap();
    let previous_hook = format!(
        "{}\n# legacy dirty .canon check removed from current hook\n",
        DEFAULT_PRE_COMMIT_HOOK
    );
    fs::write(&hook_path, previous_hook).unwrap();

    let err = run_hook_install(&root).unwrap_err();

    assert!(err.contains("Can't safely install pre-commit hook"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_install_refuses_nonstandard_git_hooks_path() {
    let root = temp_home("hook-install-nonstandard");
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .arg("init")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .arg("config")
        .arg("--local")
        .arg("core.hooksPath")
        .arg(".githooks")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let err = run_hook_install(&root).unwrap_err();

    assert!(err.contains("Can't safely install pre-commit hook"));
    assert!(!managed_pre_commit_hook_path(&root).exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_install_refuses_bare_git_repository() {
    let root = temp_home("hook-install-bare");
    let output = Command::new("git")
        .arg("init")
        .arg("--bare")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let err = run_hook_install(&root).unwrap_err();

    assert!(err.contains("requires a Git worktree"));
    assert!(!root.join(".git/canon/hooks/pre-commit").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_install_refuses_existing_default_pre_commit_hook() {
    let root = git_project("hook-install-default-existing");
    let default_hook = root.join(".git/hooks/pre-commit");
    fs::create_dir_all(default_hook.parent().unwrap()).unwrap();
    fs::write(&default_hook, "custom default hook").unwrap();

    let err = run_hook_install(&root).unwrap_err();

    assert!(err.contains("Can't safely install pre-commit hook"));
    assert!(!managed_pre_commit_hook_path(&root).exists());
    assert_eq!(current_git_hooks_path_for_worktree(&root).unwrap(), None);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_install_refuses_other_existing_default_git_hooks() {
    let root = git_project("hook-install-default-commit-msg-existing");
    let default_hook = root.join(".git/hooks/commit-msg");
    fs::create_dir_all(default_hook.parent().unwrap()).unwrap();
    fs::write(&default_hook, "custom commit-msg hook").unwrap();

    let err = run_hook_install(&root).unwrap_err();

    assert!(err.contains("Can't safely install pre-commit hook"));
    assert!(!managed_pre_commit_hook_path(&root).exists());
    assert_eq!(current_git_hooks_path_for_worktree(&root).unwrap(), None);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_install_refuses_different_existing_pre_commit_hook() {
    let root = git_project("hook-install-existing");
    let hook_path = managed_pre_commit_hook_path(&root);
    fs::create_dir_all(hook_path.parent().unwrap()).unwrap();
    fs::write(&hook_path, "custom hook").unwrap();

    let err = run_hook_install(&root).unwrap_err();
    assert!(err.contains("Can't safely install pre-commit hook"));
    assert!(!root.join(CHECK_PATH).exists());
    assert!(!root.join(".gitignore").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_uninstall_removes_reusable_hook_and_unsets_hooks_path() {
    let root = git_project("hook-uninstall");
    run_hook_install(&root).unwrap();
    let hook_path = managed_pre_commit_hook_path(&root);

    run_hook_uninstall(&root).unwrap();

    assert!(!hook_path.exists());
    assert!(!root.join(".git/hooks/pre-commit").exists());
    assert_eq!(current_git_hooks_path_for_worktree(&root).unwrap(), None);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_uninstall_recognizes_normalized_canon_hooks_path() {
    let root = git_project("hook-uninstall-normalized-hooks-path");
    run_hook_install(&root).unwrap();
    let hook_path = managed_pre_commit_hook_path(&root);
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args([
            "config",
            "--local",
            "core.hooksPath",
            ".git/../.git/canon/hooks",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    run_hook_uninstall(&root).unwrap();

    assert!(!hook_path.exists());
    assert_eq!(current_git_hooks_path_for_worktree(&root).unwrap(), None);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_uninstall_unsets_duplicate_canon_hooks_path_values() {
    let root = git_project("hook-uninstall-duplicate-hooks-path");
    run_hook_install(&root).unwrap();
    let hook_path = managed_pre_commit_hook_path(&root);
    let configured = current_git_hooks_path_for_worktree(&root).unwrap().unwrap();
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["config", "--local", "--add", "core.hooksPath", &configured])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    run_hook_uninstall(&root).unwrap();

    assert!(!hook_path.exists());
    assert_eq!(current_git_hooks_path_for_worktree(&root).unwrap(), None);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_uninstall_refuses_mixed_hooks_path_values() {
    let root = git_project("hook-uninstall-mixed-hooks-path");
    run_hook_install(&root).unwrap();
    let hook_path = managed_pre_commit_hook_path(&root);
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args([
            "config",
            "--local",
            "--add",
            "core.hooksPath",
            ".git/custom-hooks",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let err = run_hook_uninstall(&root).unwrap_err();

    assert!(err.contains("Can't safely install pre-commit hook"));
    assert!(hook_path.exists());
    assert_hooks_path_resolves_to_managed(&root);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_uninstall_refuses_to_activate_default_pre_commit_hook() {
    let root = git_project("hook-uninstall-default-existing");
    run_hook_install(&root).unwrap();
    let hook_path = managed_pre_commit_hook_path(&root);
    let default_hook = root.join(".git/hooks/pre-commit");
    fs::create_dir_all(default_hook.parent().unwrap()).unwrap();
    fs::write(&default_hook, "custom default hook").unwrap();

    let err = run_hook_uninstall(&root).unwrap_err();

    assert!(err.contains("Can't safely install pre-commit hook"));
    assert!(hook_path.exists());
    assert_hooks_path_resolves_to_managed(&root);
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn hook_uninstall_keeps_hook_when_unset_fails() {
    use std::os::unix::fs::PermissionsExt;

    let root = git_project("hook-uninstall-unset-fails");
    run_hook_install(&root).unwrap();
    let hook_path = managed_pre_commit_hook_path(&root);
    let git_dir = root.join(".git");
    let original_permissions = fs::metadata(&git_dir).unwrap().permissions();
    let mut readonly = original_permissions.clone();
    readonly.set_mode(0o555);
    fs::set_permissions(&git_dir, readonly).unwrap();

    let err = run_hook_uninstall(&root).unwrap_err();

    fs::set_permissions(&git_dir, original_permissions).unwrap();
    assert!(err.contains("failed to unset git core.hooksPath"));
    assert!(hook_path.exists());
    assert_hooks_path_resolves_to_managed(&root);
    assert!(!root.join(".git/hooks/pre-commit").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_uninstall_removes_interrupted_temporary_default_fallback() {
    let root = git_project("hook-uninstall-interrupted-fallback");
    run_hook_install(&root).unwrap();
    let hook_path = managed_pre_commit_hook_path(&root);
    let default_hook = root.join(".git/hooks/pre-commit");
    fs::create_dir_all(default_hook.parent().unwrap()).unwrap();
    fs::write(&default_hook, uninstall_fallback_pre_commit_hook_content()).unwrap();
    unset_git_hooks_path(&root).unwrap();
    fs::remove_file(&hook_path).unwrap();

    run_hook_uninstall(&root).unwrap();

    assert!(!default_hook.exists());
    assert_eq!(current_git_hooks_path_for_worktree(&root).unwrap(), None);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_uninstall_removes_interrupted_fallback_with_managed_hook_remaining() {
    let root = git_project("hook-uninstall-interrupted-fallback-managed");
    run_hook_install(&root).unwrap();
    let hook_path = managed_pre_commit_hook_path(&root);
    let default_hook = root.join(".git/hooks/pre-commit");
    fs::create_dir_all(default_hook.parent().unwrap()).unwrap();
    fs::write(&default_hook, uninstall_fallback_pre_commit_hook_content()).unwrap();
    unset_git_hooks_path(&root).unwrap();

    run_hook_uninstall(&root).unwrap();

    assert!(!hook_path.exists());
    assert!(!default_hook.exists());
    assert_eq!(current_git_hooks_path_for_worktree(&root).unwrap(), None);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_uninstall_fallback_cleanup_refuses_changed_hook() {
    let root = git_project("hook-uninstall-fallback-changed");
    let default_hook = root.join(".git/hooks/pre-commit");
    fs::create_dir_all(default_hook.parent().unwrap()).unwrap();
    fs::write(&default_hook, "custom hook").unwrap();

    let err = remove_uninstall_fallback_pre_commit_hook(&default_hook).unwrap_err();

    assert!(err.contains("content changed"));
    assert_eq!(fs::read_to_string(&default_hook).unwrap(), "custom hook");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_uninstall_fallback_cleanup_restores_hook_moved_after_race() {
    let root = git_project("hook-uninstall-fallback-race-restore");
    let default_hook = root.join(".git/hooks/pre-commit");
    let temp_dir = root.join(".git/hooks/.canon-uninstall-fallback-test");
    let moved_hook = temp_dir.join("pre-commit");
    fs::create_dir_all(&temp_dir).unwrap();
    fs::write(&moved_hook, "custom hook").unwrap();

    let err =
        remove_moved_uninstall_fallback_pre_commit_hook(&default_hook, &moved_hook, &temp_dir)
            .unwrap_err();

    assert!(err.contains("content changed"));
    assert_eq!(fs::read_to_string(&default_hook).unwrap(), "custom hook");
    assert!(!moved_hook.exists());
    assert!(!temp_dir.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_uninstall_managed_hook_cleanup_restores_hook_moved_after_race() {
    let root = git_project("hook-uninstall-managed-race-restore");
    let hook_path = managed_pre_commit_hook_path(&root);
    let temp_dir = hook_path.parent().unwrap().join(".canon-managed-hook-test");
    let moved_hook = temp_dir.join("pre-commit");
    fs::create_dir_all(&temp_dir).unwrap();
    fs::write(&moved_hook, "custom hook").unwrap();

    let err =
        remove_moved_reusable_pre_commit_hook(&hook_path, &moved_hook, &temp_dir).unwrap_err();

    assert!(err.contains("content changed"));
    assert_eq!(fs::read_to_string(&hook_path).unwrap(), "custom hook");
    assert!(!moved_hook.exists());
    assert!(!temp_dir.exists());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn hook_uninstall_managed_hook_remove_failure_restores_moved_hook() {
    use std::os::unix::fs::PermissionsExt;

    let root = git_project("hook-uninstall-managed-remove-failure-restore");
    let hook_path = managed_pre_commit_hook_path(&root);
    fs::create_dir_all(hook_path.parent().unwrap()).unwrap();
    let temp_dir = hook_path
        .parent()
        .unwrap()
        .join(".canon-managed-hook-remove-fails");
    let moved_hook = temp_dir.join("pre-commit");
    fs::create_dir_all(&temp_dir).unwrap();
    fs::write(&moved_hook, DEFAULT_PRE_COMMIT_HOOK).unwrap();
    let original_permissions = fs::metadata(&temp_dir).unwrap().permissions();
    let mut readonly = original_permissions.clone();
    readonly.set_mode(0o555);
    fs::set_permissions(&temp_dir, readonly).unwrap();

    let err =
        remove_moved_reusable_pre_commit_hook(&hook_path, &moved_hook, &temp_dir).unwrap_err();

    fs::set_permissions(&temp_dir, original_permissions).unwrap();
    assert!(err.contains("restored managed pre-commit hook"), "{err}");
    assert_eq!(
        fs::read_to_string(&hook_path).unwrap(),
        DEFAULT_PRE_COMMIT_HOOK
    );
    assert!(moved_hook.exists());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn hook_uninstall_restores_hooks_path_when_remove_fails() {
    use std::os::unix::fs::PermissionsExt;

    let root = git_project("hook-uninstall-remove-fails");
    run_hook_install(&root).unwrap();
    let hook_path = managed_pre_commit_hook_path(&root);
    let hook_dir = hook_path.parent().unwrap();
    let original_permissions = fs::metadata(hook_dir).unwrap().permissions();
    let mut readonly = original_permissions.clone();
    readonly.set_mode(0o555);
    fs::set_permissions(hook_dir, readonly).unwrap();

    let err = run_hook_uninstall(&root).unwrap_err();

    fs::set_permissions(hook_dir, original_permissions).unwrap();
    assert!(err.contains("failed to prepare managed pre-commit hook"));
    assert!(hook_path.exists());
    assert_hooks_path_resolves_to_managed(&root);
    assert!(!root.join(".git/hooks/pre-commit").exists());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn hook_install_refuses_symlinked_reusable_pre_commit_hook() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let root = git_project("hook-install-symlink");
    let target_root = temp_home("hook-install-symlink-target");
    let target = target_root.join("outside-pre-commit");
    fs::write(&target, DEFAULT_PRE_COMMIT_HOOK).unwrap();
    let hook_path = managed_pre_commit_hook_path(&root);
    fs::create_dir_all(hook_path.parent().unwrap()).unwrap();
    symlink(&target, &hook_path).unwrap();

    let err = run_hook_install(&root).unwrap_err();

    assert!(err.contains("refusing to use symlinked pre-commit hook"));
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        DEFAULT_PRE_COMMIT_HOOK
    );
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o111,
        0
    );
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(target_root);
}

#[test]
fn hook_install_uses_git_path_in_linked_worktree() {
    let root = git_project("hook-install-linked-main");
    commit_all(&root, "initial");
    let linked = temp_home("hook-install-linked-worktree");
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["worktree", "add", "--detach"])
        .arg(&linked)
        .arg("HEAD")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    run_hook_install(&linked).unwrap();

    let hook_path = managed_pre_commit_hook_path(&linked);
    assert_eq!(
        fs::read_to_string(&hook_path).unwrap(),
        DEFAULT_PRE_COMMIT_HOOK
    );
    assert_hooks_path_resolves_to_managed(&linked);
    assert!(!linked.join(".git/canon/hooks/pre-commit").exists());
    let _ = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["worktree", "remove", "--force"])
        .arg(&linked)
        .output();
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(linked);
}

#[test]
fn hook_install_refuses_default_hook_in_linked_worktree_git_dir() {
    let root = git_project("hook-install-linked-default-main");
    commit_all(&root, "initial");
    let linked = temp_home("hook-install-linked-default-worktree");
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["worktree", "add", "--detach"])
        .arg(&linked)
        .arg("HEAD")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let default_hook = resolve_git_path(&linked, "hooks/pre-commit").unwrap();
    fs::create_dir_all(default_hook.parent().unwrap()).unwrap();
    fs::write(&default_hook, "custom linked worktree hook").unwrap();

    let err = run_hook_install(&linked).unwrap_err();

    assert!(err.contains("Can't safely install pre-commit hook"));
    assert!(!managed_pre_commit_hook_path(&linked).exists());
    assert_eq!(current_git_hooks_path_for_worktree(&linked).unwrap(), None);
    let _ = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["worktree", "remove", "--force"])
        .arg(&linked)
        .output();
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(linked);
}

#[test]
fn hook_uninstall_uses_default_common_hook_path_in_linked_worktree() {
    let root = git_project("hook-uninstall-linked-main");
    commit_all(&root, "initial");
    let linked = temp_home("hook-uninstall-linked-worktree");
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["worktree", "add", "--detach"])
        .arg(&linked)
        .arg("HEAD")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    run_hook_install(&linked).unwrap();
    let managed_hook = managed_pre_commit_hook_path(&linked);
    assert_eq!(
        resolve_git_path(&linked, "hooks/pre-commit").unwrap(),
        managed_hook
    );
    let default_hook = root.join(".git/hooks/pre-commit");

    run_hook_uninstall(&linked).unwrap();

    assert!(!managed_hook.exists());
    assert!(!default_hook.exists());
    assert_eq!(current_git_hooks_path_for_worktree(&linked).unwrap(), None);
    let _ = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["worktree", "remove", "--force"])
        .arg(&linked)
        .output();
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(linked);
}

fn managed_pre_commit_hook_path(root: &Path) -> PathBuf {
    resolve_git_path(root, PRE_COMMIT_HOOK_PATH).unwrap()
}

fn managed_git_hooks_path(root: &Path) -> PathBuf {
    resolve_git_path(root, GIT_HOOKS_PATH).unwrap()
}

fn assert_hooks_path_resolves_to_managed(root: &Path) {
    let configured = PathBuf::from(current_git_hooks_path_for_worktree(root).unwrap().unwrap());
    let resolved = if configured.is_absolute() {
        configured
    } else {
        root.join(configured)
    };
    assert_eq!(resolved, managed_git_hooks_path(root));
}
