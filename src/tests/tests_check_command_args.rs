use super::*;

#[test]
fn check_agent_message_requires_effective_default_inputs() {
    let staged = TreeSource::Staged;
    let head = TreeSource::Git {
        treeish: "HEAD".to_string(),
        tree_oid: String::new(),
    };
    assert!(check_command_writes_agent_message(
        Path::new(CHECK_PATH),
        &staged,
        &head,
        false
    ));
    assert!(!check_command_writes_agent_message(
        Path::new(CHECK_PATH),
        &staged,
        &head,
        true
    ));
    assert!(!check_command_writes_agent_message(
        Path::new("alt.yml"),
        &staged,
        &head,
        false
    ));
    assert!(!check_command_writes_agent_message(
        Path::new(CHECK_PATH),
        &TreeSource::Git {
            treeish: "HEAD".to_string(),
            tree_oid: "tree".to_string(),
        },
        &head,
        false
    ));
}

#[test]
fn check_help_is_detected_with_other_options() {
    assert!(check_help_requested(&["--help".into()]));
    assert!(check_help_requested(&[
        "--no-sandbox".into(),
        "--help".into()
    ]));
    assert!(!check_help_requested(&["--no-sandbox".into()]));
}

#[test]
fn check_help_omits_internal_break_after_tokens_option() {
    let mut command = check_help_command();
    let help = command.render_help().to_string();

    assert!(!help.contains("--break-after-tokens"));
}

#[test]
fn check_command_accepts_custom_config_option() {
    let parsed = parse_check_command_args(&[
        "--config".into(),
        "alt.yml".into(),
        "--keep-going".into(),
        "2".into(),
    ])
    .unwrap();
    assert_eq!(parsed.config_path, PathBuf::from("alt.yml"));
    assert!(parsed.options.keep_going);
    assert_eq!(parsed.options.selectors, vec![OsString::from("2")]);

    let parsed = parse_check_command_args(&["-c".into(), "old.yml".into()]).unwrap();
    assert_eq!(parsed.config_path, PathBuf::from("old.yml"));

    let parsed = parse_check_command_args(&["--config=old.yml".into()]).unwrap();
    assert_eq!(parsed.config_path, PathBuf::from("old.yml"));

    assert!(parse_check_command_args(&["-c".into()]).is_err());
    assert!(
        parse_check_command_args(&["-c".into(), "a.yml".into(), "--config=b.yml".into()]).is_err()
    );
    assert!(parse_check_command_args(&["-c".into(), "../outside.yml".into()]).is_err());
    assert!(parse_check_command_args(&["-c".into(), "/tmp/outside.yml".into()]).is_err());
}

#[test]
fn check_command_accepts_attached_short_option_values() {
    let parsed = parse_check_command_args(&["-calt.yml".into()]).unwrap();
    assert_eq!(parsed.config_path, PathBuf::from("alt.yml"));

    let parsed = parse_check_command_args(&["-qQuestion?".into(), "-ssrc".into()]).unwrap();
    assert_eq!(parsed.config_path, PathBuf::from(CHECK_PATH));
    assert_eq!(parsed.query.as_deref(), Some("Question?"));
    assert_eq!(parsed.query_scope, vec!["src".to_string()]);
    assert!(parsed.options.is_empty());
}

#[test]
fn check_command_accepts_query_mode() {
    let parsed = parse_check_command_args(&["-q".into(), "Question?".into()]).unwrap();
    assert_eq!(parsed.query.as_deref(), Some("Question?"));
    assert!(parsed.query_scope.is_empty());
    assert_eq!(parsed.config_path, PathBuf::from(CHECK_PATH));
    assert!(parsed.options.is_empty());

    let parsed = parse_check_command_args(&[
        "--config".into(),
        "alt.yml".into(),
        "-q".into(),
        "Question?".into(),
    ])
    .unwrap();
    assert_eq!(parsed.config_path, PathBuf::from("alt.yml"));
    assert_eq!(parsed.query.as_deref(), Some("Question?"));
    assert!(parsed.query_scope.is_empty());

    assert!(parse_check_command_args(&["-q".into()]).is_err());
    assert!(parse_check_command_args(&[
        "-q".into(),
        "Question?".into(),
        "-q".into(),
        "Again?".into()
    ])
    .is_err());
    assert!(parse_check_command_args(&["-q".into(), "Question?".into(), "1".into()]).is_err());
    assert!(
        parse_check_command_args(&["-q".into(), "Question?".into(), "--ignore-cache".into()])
            .is_err()
    );
    assert!(parse_check_command_args(&["-q".into(), "Question?".into(), "--all".into()]).is_err());
    assert!(
        parse_check_command_args(&["-q".into(), "Question?".into(), "--keep-going".into()])
            .is_err()
    );
}

#[test]
fn check_command_accepts_tree_and_sandbox_options() {
    let parsed = parse_check_command_args(&[
        "--tree".into(),
        "HEAD".into(),
        "--against-tree".into(),
        "HEAD~1".into(),
        "--no-sandbox".into(),
    ])
    .unwrap();

    assert_eq!(parsed.tree, "HEAD");
    assert_eq!(parsed.against_tree, "HEAD~1");
    assert!(parsed.against_tree_explicit);
    assert!(parsed.no_sandbox);

    let parsed = parse_check_command_args(&[]).unwrap();
    assert_eq!(parsed.tree, ":staged");
    assert_eq!(parsed.against_tree, "HEAD");
    assert!(!parsed.against_tree_explicit);
    assert!(!parsed.no_sandbox);

    assert!(parse_check_command_args(&["--tree".into(), ":worktree".into()]).is_err());
    assert!(parse_check_command_args(&["--against-tree".into(), ":worktree".into()]).is_err());
}

#[test]
fn check_command_accepts_query_scope_option() {
    let parsed = parse_check_command_args(&[
        "-s".into(),
        "./src".into(),
        "--scope".into(),
        "tests".into(),
        "-q".into(),
        "Question?".into(),
        "--scope=README.md".into(),
    ])
    .unwrap();

    assert_eq!(parsed.query.as_deref(), Some("Question?"));
    assert_eq!(
        parsed.query_scope,
        vec![
            "src".to_string(),
            "tests".to_string(),
            "README.md".to_string()
        ]
    );
    assert!(parsed.options.is_empty());

    assert!(parse_check_command_args(&["-q".into(), "Question?".into(), "-s".into()]).is_err());
    assert!(
        parse_check_command_args(&["-q".into(), "Question?".into(), "--scope=".into()]).is_err()
    );
    assert!(
        parse_check_command_args(&["-q".into(), "Question?".into(), "--scope".into()]).is_err()
    );
    assert!(parse_check_command_args(&[
        "-q".into(),
        "Question?".into(),
        "-s".into(),
        "../src".into()
    ])
    .is_err());
    assert!(parse_check_command_args(&["-s".into(), "src".into()]).is_err());
}
