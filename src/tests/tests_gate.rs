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

    let result = run_gate_command(&root, &[OsString::from(expectation.display_id.clone())]);

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

    let config = parse_check_config(check_config_yaml()).unwrap();
    let result = run_gate_command(&root, &[OsString::from(test_selector(&config, "1"))]);

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
    let head_hash = gate_head_tree_fingerprint(&root, &full_scope())
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

    let result = run_gate_command(&root, &[OsString::from(expectation.display_id.clone())]);

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
fn gate_missing_cache_advice_prioritizes_regressions() {
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
    let head_hash = gate_head_tree_fingerprint(&root, &full_scope())
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
        &[],
        GateCaches {
            history: &mut history_cache,
            scope_hash: &mut scope_hash_cache,
        },
        unix_timestamp().unwrap(),
        |event| {
            match event {
                GateFailureEvent::Regressed(record) => {
                    events.push(format!("regressed:{}", record.display_id));
                }
                GateFailureEvent::Missing(expectation) => {
                    events.push(format!("missing:{}", expectation.display_id));
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
    assert_eq!(
        events,
        vec![format!("regressed:{}", expectations[1].display_id)]
    );
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

    let config = parse_check_config(check_config_yaml()).unwrap();
    let result = run_gate_command(&root, &[OsString::from(test_selector(&config, "1"))]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_explicit_selection_uses_cooldown_filtered_selected_set() {
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

    let result = run_gate_command(&root, &[OsString::from(expectation.display_id.clone())]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_default_selection_skips_fresh_cooldown_expectation() {
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

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_explicit_selection_removes_fresh_cooldown_pass_before_regression_loop() {
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
    let head_hash = gate_head_tree_fingerprint(&root, &full_scope())
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

    let result = run_gate_command(&root, &[OsString::from(expectation.display_id.clone())]);

    assert!(result.is_ok());
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

    let result = run_gate_command(&root, &[OsString::from(expectation.display_id.clone())]);

    assert!(result.is_ok());
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
    let head_hash = gate_head_tree_fingerprint(&root, &full_scope())
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

    let result = run_gate_command(&root, &[OsString::from(expectation.display_id.clone())]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}
