use super::*;

#[test]
fn init_creates_template_and_fails_when_existing() {
    let root = temp_home("init");
    run_init(&root).unwrap();
    let check_path = root.join(CHECK_PATH);
    assert_eq!(
        fs::read_to_string(&check_path).unwrap(),
        DEFAULT_CHECK_TEMPLATE
    );
    assert!(!root.join(".gitignore").exists());
    assert!(!root.join(PRE_COMMIT_HOOK_PATH).exists());
    assert!(run_init(&root).is_err());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn init_refuses_symlinked_check_path_without_overwriting_target() {
    use std::os::unix::fs::symlink;

    let root = temp_home("init-symlink-check");
    let target_root = temp_home("init-symlink-check-target");
    let target = target_root.join("outside-check.yml");
    fs::write(&target, "outside\n").unwrap();
    fs::create_dir_all(root.join(".canon")).unwrap();
    symlink(&target, root.join(CHECK_PATH)).unwrap();

    let err = run_init(&root).unwrap_err();

    assert!(err.contains("already exists"));
    assert_eq!(fs::read_to_string(&target).unwrap(), "outside\n");
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(target_root);
}

#[test]
fn init_does_not_require_thread_id() {
    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CODEX_THREAD_ID"]);
    env_snapshot.remove("CODEX_THREAD_ID");
    let root = temp_home("init-no-thread");
    run_init(&root).unwrap();
    assert!(root.join(CHECK_PATH).exists());
    let _ = fs::remove_dir_all(root);
}
