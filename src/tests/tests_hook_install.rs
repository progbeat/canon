use super::*;

#[test]
fn hook_install_creates_reusable_pre_commit_hook() {
    let root = git_project("hook-install");
    run_hook_install(&root).unwrap();
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
    assert!(!root.join(CHECK_PATH).exists());
    assert!(!root.join(".gitignore").exists());
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
fn hook_install_refuses_non_exact_existing_canon_pre_commit_hook() {
    let root = temp_home("hook-install-update");
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
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
    assert!(!root.join(PRE_COMMIT_HOOK_PATH).exists());
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
    assert!(!root.join(PRE_COMMIT_HOOK_PATH).exists());
    assert_eq!(current_git_hooks_path_for_worktree(&root).unwrap(), None);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_install_refuses_different_existing_pre_commit_hook() {
    let root = temp_home("hook-install-existing");
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
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

    run_hook_uninstall(&root).unwrap();

    assert!(!root.join(PRE_COMMIT_HOOK_PATH).exists());
    assert_eq!(current_git_hooks_path_for_worktree(&root).unwrap(), None);
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
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
    fs::create_dir_all(hook_path.parent().unwrap()).unwrap();
    symlink(&target, &hook_path).unwrap();

    let err = run_hook_install(&root).unwrap_err();

    assert!(err.contains("refusing to chmod symlink"));
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
