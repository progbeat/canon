use super::*;

#[test]
fn aliases_work() {
    with_env("aliases", |_| {
        run(vec![]).unwrap();
        run(vec!["pwd".into()]).unwrap();
        run(vec!["p".into(), "file.rs".into()]).unwrap();
        run(vec!["path".into(), "file.rs".into()]).unwrap();
        run(vec!["w".into(), "file.rs".into(), "body".into()]).unwrap();
        run(vec!["a".into(), "file.rs".into(), "more".into()]).unwrap();
        run(vec!["read".into(), "file.rs".into()]).unwrap();
        run(vec!["d".into(), "file.rs".into()]).unwrap();
        assert!(run(vec!["-r".into()]).is_err());
        assert!(run(vec!["file.rs".into()]).is_err());
    });
}

#[test]
fn unknown_commands_are_reported_before_config_load() {
    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CANON_HOME", "CODEX_THREAD_ID"]);
    let home = TestDir::new("unknown-command");
    env_snapshot.remove("CODEX_THREAD_ID");
    env_snapshot.set("CANON_HOME", home.path());

    assert_eq!(
        run(vec!["typo".into()]).unwrap_err(),
        CommandError::UnknownCommand("typo".to_string())
    );
    assert_eq!(
        run(vec!["--typo".into()]).unwrap_err(),
        CommandError::UnknownOption("--typo".to_string())
    );
}
