use super::*;

#[test]
fn missing_thread_id_fails() {
    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CANON_HOME", "CODEX_THREAD_ID"]);
    let home = TestDir::new("missing-thread");
    env_snapshot.remove("CODEX_THREAD_ID");
    env_snapshot.set("CANON_HOME", home.path());
    let result = Config::from_env();
    assert!(result.is_err());
}

#[test]
fn unsafe_thread_id_segments_fail() {
    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CANON_HOME", "CODEX_THREAD_ID"]);
    let home = TestDir::new("unsafe-thread");
    env_snapshot.set("CANON_HOME", home.path());

    env_snapshot.set("CODEX_THREAD_ID", "..");
    assert!(Config::from_env().is_err());

    env_snapshot.set("CODEX_THREAD_ID", ".");
    assert!(Config::from_env().is_err());
}

#[test]
fn test_home_uses_git_canon_layout() {
    with_env("home-override", |home| {
        let config = Config::from_env().unwrap();
        assert_eq!(
            config.root,
            home.join(".git")
                .join("canon")
                .join("codex")
                .join("thread-test")
        );
    });
}

#[test]
fn project_root_uses_git_canon_state_dir() {
    let root = git_project("config-git-state");
    let config = Config::for_project_thread(&root, "thread-test").unwrap();
    assert_eq!(
        config.root,
        root.join(".git")
            .join("canon")
            .join("codex")
            .join("thread-test")
    );
}
