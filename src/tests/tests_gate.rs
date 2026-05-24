use super::*;

#[test]
fn gate_passes_with_current_cached_pass() {
    let root = git_project("gate-pass");
    commit_all(&root, "initial");
    write_check_config(&root);
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add check config");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let scope_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &expectation_record(&config.agent, &expectation, "pass", "yes", scope_hash),
    )
    .unwrap();

    let result = run_gate_command(&root, &[]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_passes_when_cache_is_missing_without_head_pass() {
    let root = git_project("gate-missing");
    commit_all(&root, "initial");
    write_check_config(&root);
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add check config");

    let result = run_gate_command(&root, &[]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_fails_when_head_pass_has_no_current_cache() {
    let root = git_project("gate-head-pass-current-missing");
    commit_all(&root, "initial");
    write_check_config(&root);
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add check config");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let head_hash = gate_head_tree_fingerprint(&root, &config.agent, &full_scope())
        .unwrap()
        .unwrap();
    append_history_record(
        &root,
        &expectation,
        &expectation_record(&config.agent, &expectation, "pass", "yes", head_hash),
    )
    .unwrap();
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();

    let result = run_gate_command(&root, &[]);

    assert_eq!(result.unwrap_err(), CommandError::GateFailed);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_failed_error_display_is_descriptive() {
    assert_eq!(CommandError::GateFailed.to_string(), "canon gate failed");
    assert!(!command_error_has_public_diagnostic(
        &CommandError::GateFailed
    ));
    assert!(!command_error_has_public_diagnostic(
        &CommandError::CheckFailed
    ));
}

#[test]
fn gate_rejects_arguments() {
    let err = run_gate_command(Path::new("."), &[OsString::from("1")]).unwrap_err();

    assert_eq!(
        err.to_string(),
        "canon gate does not accept arguments\n▷ Run `canon gate` without arguments."
    );
}

#[test]
fn gate_missing_cache_advice_prioritizes_regressions() {
    assert_eq!(
        gate_regression_advice(),
        "▷ Fix staged regressions and run `canon check` again!"
    );
    assert_eq!(
        gate_missing_cache_advice(false),
        Some("canon gate: run `canon check` before committing")
    );
    assert_eq!(
        gate_missing_cache_advice(true),
        Some("canon gate: fix staged regressions before filling missing cache")
    );
}

#[test]
fn gate_ignores_missing_cache_before_regression() {
    let root = git_project("gate-missing-before-regression");
    commit_all(&root, "initial");
    write_check_config(&root);
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add check config");
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    let config = parse_check_config(check_config_yaml()).unwrap();
    let identities = expectation_identities(&config).unwrap();
    let expectations = check_options(&config, &[], false, true).selected;
    let head_hash = gate_head_tree_fingerprint(&root, &config.agent, &full_scope())
        .unwrap()
        .unwrap();
    append_history_record(
        &root,
        &expectations[1],
        &expectation_record(&config.agent, &expectations[1], "pass", "no", head_hash),
    )
    .unwrap();
    let current_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectations[1],
        &expectation_record(&config.agent, &expectations[1], "fail", "yes", current_hash),
    )
    .unwrap();
    let mut history_cache = HistoryCache::new();
    let mut scope_hash_cache = ScopeHashCache::new();
    let mut events = Vec::new();

    let passed = gate_pass_with_config(
        &root,
        &config,
        &identities,
        GateCaches {
            history: &mut history_cache,
            scope_hash: &mut scope_hash_cache,
        },
        unix_timestamp().unwrap(),
        |event| {
            match event {
                GateFailureEvent::Regressed => {
                    events.push("regressed".to_string());
                }
                GateFailureEvent::Missing => {
                    events.push("missing".to_string());
                }
                GateFailureEvent::MissingComplete { has_regressions } => {
                    events.push(format!("complete:{has_regressions}"));
                }
            }
            Ok(())
        },
    )
    .unwrap();

    assert!(!passed);
    assert_eq!(events, vec!["regressed"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_passes_canon_only_change_without_checking_cache() {
    let root = git_project("gate-canon-only");
    commit_all(&root, "initial");
    write_check_config(&root);
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();

    let result = run_gate_command(&root, &[]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_canon_path_classification_matches_canon_subtree_only() {
    assert!(is_canon_project_path_bytes(b".canon/check.yml"));
    assert!(is_canon_project_path_bytes(b".canon/draft/spec.md"));
    assert!(!is_canon_project_path_bytes(b".canon"));
    assert!(!is_canon_project_path_bytes(b".canonical/file.md"));
}

#[test]
fn gate_passes_canon_only_change_without_loading_config() {
    let root = git_project("gate-canon-only-invalid-config");
    commit_all(&root, "initial");
    fs::create_dir_all(root.join(".canon")).unwrap();
    fs::write(root.join(CHECK_PATH), "not: [valid").unwrap();
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();

    let result = run_gate_command(&root, &[]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_passes_canon_only_deletion_without_loading_config() {
    let root = git_project("gate-canon-only-delete");
    commit_all(&root, "initial");
    fs::create_dir_all(root.join(".canon")).unwrap();
    fs::write(root.join(CHECK_PATH), "not: [valid").unwrap();
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add invalid canon config");
    fs::remove_file(root.join(CHECK_PATH)).unwrap();
    Command::new("git")
        .args(["add", "-A", ".canon"])
        .current_dir(&root)
        .output()
        .unwrap();

    let result = run_gate_command(&root, &[]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_fails_mixed_canon_and_non_canon_change() {
    let root = git_project("gate-mixed-canon");
    commit_all(&root, "initial");
    write_check_config(&root);
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .args(["add", ".canon/check.yml", "README.md"])
        .current_dir(&root)
        .output()
        .unwrap();

    let result = run_gate_command(&root, &[]);

    assert_eq!(result.unwrap_err(), CommandError::GateFailed);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_passes_non_canon_change_with_missing_cache_without_head_pass() {
    let root = git_project("gate-non-canon-missing");
    commit_all(&root, "initial");
    write_check_config(&root);
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add check config");
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();

    let result = run_gate_command(&root, &[]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_checks_fresh_cooldown_expectation_missing_cache() {
    let root = git_project("gate-cooldown-regression");
    commit_all(&root, "initial");
    let yaml = r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#;
    fs::create_dir_all(root.join(".canon")).unwrap();
    fs::write(root.join(CHECK_PATH), yaml).unwrap();
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add check config");
    let config = parse_check_config(yaml).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let old_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    let mut record =
        expectation_record(&config.agent, &expectation, "pass", "yes", old_hash.clone());
    record.timestamp = format_record_timestamp(unix_timestamp().unwrap());
    append_history_record(&root, &expectation, &record).unwrap();
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    let current_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    assert_ne!(current_hash, old_hash);

    let result = run_gate_command(&root, &[]);

    assert_eq!(result.unwrap_err(), CommandError::GateFailed);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_does_not_skip_fresh_cooldown_expectation() {
    let root = git_project("gate-default-cooldown");
    commit_all(&root, "initial");
    let yaml = r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#;
    fs::create_dir_all(root.join(".canon")).unwrap();
    fs::write(root.join(CHECK_PATH), yaml).unwrap();
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add check config");
    let config = parse_check_config(yaml).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let old_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    let mut record = expectation_record(&config.agent, &expectation, "pass", "yes", old_hash);
    record.timestamp = format_record_timestamp(unix_timestamp().unwrap());
    append_history_record(&root, &expectation, &record).unwrap();
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();

    let result = run_gate_command(&root, &[]);

    assert_eq!(result.unwrap_err(), CommandError::GateFailed);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_checks_regression_even_with_fresh_cooldown_pass() {
    let root = git_project("gate-cooldown-regression-over-pass");
    commit_all(&root, "initial");
    let yaml = r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#;
    fs::create_dir_all(root.join(".canon")).unwrap();
    fs::write(root.join(CHECK_PATH), yaml).unwrap();
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add check config");
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    let config = parse_check_config(yaml).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let head_hash = gate_head_tree_fingerprint(&root, &config.agent, &full_scope())
        .unwrap()
        .unwrap();
    append_history_record(
        &root,
        &expectation,
        &expectation_record(&config.agent, &expectation, "pass", "yes", head_hash),
    )
    .unwrap();
    let current_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &expectation_record(&config.agent, &expectation, "fail", "no", current_hash),
    )
    .unwrap();
    let mut pass = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        "old".to_string(),
    );
    pass.timestamp = format_record_timestamp(unix_timestamp().unwrap());
    append_history_record(&root, &expectation, &pass).unwrap();

    let result = run_gate_command(&root, &[]);

    assert_eq!(result.unwrap_err(), CommandError::GateFailed);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_passes_for_new_current_failure_without_head_pass() {
    let root = git_project("gate-new-fail");
    commit_all(&root, "initial");
    write_check_config(&root);
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add check config");
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let current_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &expectation_record(&config.agent, &expectation, "fail", "no", current_hash),
    )
    .unwrap();

    let result = run_gate_command(&root, &[]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_passes_when_same_tree_check_allows_commit_with_remaining_failure() {
    let root = git_project("gate-same-tree-check-commit");
    let yaml = r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Fixed?"
    a: "yes"
  - q: "Still failing?"
    a: "no"
"#;
    fs::create_dir_all(root.join(".canon")).unwrap();
    fs::write(root.join(CHECK_PATH), yaml).unwrap();
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add check config");
    let config = parse_check_config(yaml).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let head_hash = gate_head_tree_fingerprint(&root, &config.agent, &full_scope())
        .unwrap()
        .unwrap();
    append_history_record(
        &root,
        &options.selected[0],
        &expectation_record(
            &config.agent,
            &options.selected[0],
            "fail",
            "no",
            head_hash.clone(),
        ),
    )
    .unwrap();
    append_history_record(
        &root,
        &options.selected[1],
        &expectation_record(
            &config.agent,
            &options.selected[1],
            "fail",
            "yes",
            head_hash,
        ),
    )
    .unwrap();
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    let current_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    let current_pass = expectation_record(
        &config.agent,
        &options.selected[0],
        "pass",
        "yes",
        current_hash.clone(),
    );
    let current_fail = expectation_record(
        &config.agent,
        &options.selected[1],
        "fail",
        "yes",
        current_hash,
    );
    append_history_record(&root, &options.selected[0], &current_pass).unwrap();
    append_history_record(&root, &options.selected[1], &current_fail).unwrap();
    let report = CheckRunReport {
        records: vec![current_pass, current_fail],
        non_selected: Vec::new(),
        evaluated: 2,
        selected: 2,
        skipped: 0,
        silent: 0,
        narrowing: NarrowingStats::default(),
    };

    assert_eq!(
        check_agent_messages(
            &root,
            &config,
            &report,
            &mut HistoryCache::new(),
            &mut ScopeHashCache::new(),
        )
        .unwrap(),
        vec![
            "▷ +1 pass compared to HEAD. Commit the staged changes NOW!".to_string(),
            "▷ Then fix the remaining issues and run `canon check` again!".to_string(),
        ]
    );
    assert!(run_gate_command(&root, &[]).is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_accepts_failure_already_present_on_head() {
    let root = git_project("gate-head-fail");
    commit_all(&root, "initial");
    write_check_config(&root);
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add check config");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let head_hash = gate_head_tree_fingerprint(&root, &config.agent, &full_scope())
        .unwrap()
        .unwrap();
    append_history_record(
        &root,
        &expectation,
        &expectation_record(&config.agent, &expectation, "fail", "no", head_hash),
    )
    .unwrap();
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    let current_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &expectation_record(&config.agent, &expectation, "fail", "no", current_hash),
    )
    .unwrap();

    let result = run_gate_command(&root, &[]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}
