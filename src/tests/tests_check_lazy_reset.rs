use super::*;

#[test]
fn lazy_full_scope_reset_sets_only_sampled_narrowed_history_to_full_scope() {
    let root = git_project("check-lazy-reset-history");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectations = check_options(&config, &[], false, true).selected;
    let first_hash = staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap();
    let second_scope = vec!["README.md".to_string()];
    let second_hash = staged_visible_tree_oid(&root, &config.agent, &second_scope).unwrap();
    append_history_record(
        &root,
        &expectations[0],
        &expectation_record(&config.agent, &expectations[0], "pass", "yes", first_hash),
    )
    .unwrap();
    let mut narrowed_record = expectation_record(
        &config.agent,
        &expectations[1],
        "pass",
        "no",
        second_hash.clone(),
    );
    narrowed_record.scope = second_scope;
    append_history_record(&root, &expectations[1], &narrowed_record).unwrap();
    let reset_history_path = history_path(&root, &expectations[1]).unwrap();
    let reset_marker_path =
        full_scope_reset_marker_path_with_cache(&root, &expectations[1], &mut HistoryCache::new())
            .unwrap();

    set_non_selected_expectation_scopes_to_full(&root, &[expectations[1].clone()]).unwrap();

    assert_eq!(
        read_history_records(&root, &expectations[0]).unwrap().len(),
        1
    );
    assert!(reset_history_path.exists());
    let reset_records = read_history_records(&root, &expectations[1]).unwrap();
    assert_eq!(reset_records.len(), 1);
    assert_eq!(reset_records[0].scope, vec!["README.md".to_string()]);
    assert!(reset_marker_path.exists());
    assert_eq!(
        same_tree_history_record(&root, &config.agent, &expectations[1])
            .unwrap()
            .map(|record| record.scope),
        Some(vec!["README.md".to_string()])
    );
    let mut history_cache = HistoryCache::new();
    assert_eq!(
        latest_stored_q_scope_with_cache(
            &root,
            &config.agent,
            &expectations[1],
            &mut history_cache,
        )
        .unwrap(),
        Some(full_scope())
    );
    assert_eq!(
        latest_stored_q_scope_with_cache(
            &root,
            &config.agent,
            &expectations[1],
            &mut HistoryCache::new(),
        )
        .unwrap()
        .unwrap_or_else(full_scope),
        full_scope()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lazy_full_scope_reset_schedule_applies_on_next_check_start() {
    let root = git_project("check-lazy-reset-scheduled");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let identities = expectation_identities(&config).unwrap();
    let expectations = check_options(&config, &[], false, true).selected;
    let narrowed_scope = vec!["README.md".to_string()];
    let mut narrowed_record = expectation_record(
        &config.agent,
        &expectations[1],
        "pass",
        "no",
        staged_visible_tree_oid(&root, &config.agent, &narrowed_scope).unwrap(),
    );
    narrowed_record.scope = narrowed_scope.clone();
    append_history_record(&root, &expectations[1], &narrowed_record).unwrap();
    let reset_history_path = history_path(&root, &expectations[1]).unwrap();
    let reset_marker_path =
        full_scope_reset_marker_path_with_cache(&root, &expectations[1], &mut HistoryCache::new())
            .unwrap();

    schedule_lazy_full_scope_resets(&root, &[expectations[1].clone()]).unwrap();

    assert_eq!(
        read_history_records(&root, &expectations[1]).unwrap().len(),
        1
    );
    assert_eq!(
        apply_scheduled_lazy_full_scope_resets(&root, &config, &identities).unwrap(),
        1
    );
    assert!(reset_history_path.exists());
    assert_eq!(
        read_history_records(&root, &expectations[1]).unwrap().len(),
        1
    );
    assert!(reset_marker_path.exists());
    assert_eq!(
        apply_scheduled_lazy_full_scope_resets(&root, &config, &identities).unwrap(),
        0
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lazy_full_scope_reset_marker_clears_after_new_answer_history_append() {
    let root = git_project("check-lazy-reset-marker-clears");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let identities = expectation_identities(&config).unwrap();
    let expectations = check_options(&config, &[], false, true).selected;
    let narrowed_scope = vec!["README.md".to_string()];
    let mut narrowed_record = expectation_record(
        &config.agent,
        &expectations[1],
        "pass",
        "no",
        staged_visible_tree_oid(&root, &config.agent, &narrowed_scope).unwrap(),
    );
    narrowed_record.scope = narrowed_scope.clone();
    append_history_record(&root, &expectations[1], &narrowed_record).unwrap();
    let marker_path =
        full_scope_reset_marker_path_with_cache(&root, &expectations[1], &mut HistoryCache::new())
            .unwrap();
    schedule_lazy_full_scope_resets(&root, &[expectations[1].clone()]).unwrap();
    apply_scheduled_lazy_full_scope_resets(&root, &config, &identities).unwrap();
    assert!(marker_path.exists());

    let full_record = expectation_record(
        &config.agent,
        &expectations[1],
        "pass",
        "no",
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap(),
    );
    append_history_record(&root, &expectations[1], &full_record).unwrap();

    assert!(!marker_path.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn finish_check_report_logs_finish_after_lazy_reset_failure() {
    let root = git_project("check-lazy-reset-finish-log");
    enable_diagnostic_logs(&root);
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectations = check_options(&config, &[], false, true).selected;
    let narrowed_scope = vec!["README.md".to_string()];
    let mut narrowed_record = expectation_record(
        &config.agent,
        &expectations[1],
        "pass",
        "no",
        staged_visible_tree_oid(&root, &config.agent, &narrowed_scope).unwrap(),
    );
    narrowed_record.scope = narrowed_scope.clone();
    append_history_record(&root, &expectations[1], &narrowed_record).unwrap();
    let reset_path = resolve_git_path(&root, "canon/lazy-full-scope-reset").unwrap();
    fs::create_dir_all(&reset_path).unwrap();
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();
    let mut result_output = Vec::new();
    let mut check_caches = crate::check::CheckRunCaches::new();
    let report = CheckRunReport {
        records: Vec::new(),
        non_selected: vec![expectations[1].clone()],
        cached: vec![CachedExpectation {
            expectation: expectations[1].clone(),
            record: narrowed_record,
        }],
        evaluated: 1_000,
        selected: 0,
        skipped: 0,
        silent: 0,
        narrowing: NarrowingStats::default(),
    };

    let err = crate::check_command_finish::finish_check_report(
        crate::check_command_finish::CheckReportFinishContext {
            root: &root,
            config: &config,
            diagnostic_log: &mut diagnostic_log,
            result_output: &mut result_output,
            check_caches: &mut check_caches,
            write_agent_message: false,
        },
        &report,
        None,
    )
    .unwrap_err();

    assert!(!err.to_string().is_empty());
    let log = fs::read_to_string(root.join(".git/canon/logs/0.jsonl")).unwrap();
    assert!(log.contains(r#""event":"lazy_full_scope_reset.error""#));
    assert!(log.contains(r#""event":"check.finish""#));
    assert!(log.contains(r#""status":"error""#));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lazy_full_scope_reset_count_uses_evaluated_count_and_candidate_cap() {
    assert_eq!(lazy_full_scope_reset_count(0, 1, 5), 0);
    assert_eq!(lazy_full_scope_reset_count(128, 1, 5), 1);
    assert_eq!(lazy_full_scope_reset_count(256, 1, 5), 2);
    assert_eq!(lazy_full_scope_reset_count(1_000, 1, 3), 3);
}

#[test]
fn lazy_full_scope_reset_plan_samples_only_narrowed_history() {
    let root = git_project("check-lazy-reset-candidates");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectations = check_options(&config, &[], false, true).selected;
    let full_record = expectation_record(
        &config.agent,
        &expectations[0],
        "pass",
        "yes",
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap(),
    );
    append_history_record(&root, &expectations[0], &full_record).unwrap();
    let narrowed_scope = vec!["README.md".to_string()];
    let mut narrowed_record = expectation_record(
        &config.agent,
        &expectations[1],
        "pass",
        "no",
        staged_visible_tree_oid(&root, &config.agent, &narrowed_scope).unwrap(),
    );
    narrowed_record.scope = narrowed_scope;
    append_history_record(&root, &expectations[1], &narrowed_record).unwrap();
    let cached = vec![
        CachedExpectation {
            expectation: expectations[0].clone(),
            record: full_record,
        },
        CachedExpectation {
            expectation: expectations[1].clone(),
            record: narrowed_record,
        },
    ];

    let plan = plan_lazy_full_scope_reset(&root, &config.agent, 128, &cached, 0).unwrap();

    assert_eq!(plan.candidate_count, 1);
    assert_eq!(plan.expectations.len(), 1);
    assert_eq!(plan.expectations[0].id, expectations[1].id);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lazy_full_scope_reset_preserves_existing_full_scope_pass_when_resetting_narrowed_scope() {
    let root = git_project("check-lazy-reset-preserve-full-pass");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &[], false, true).selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &expectation_record(
            &config.agent,
            &expectation,
            "pass",
            "yes",
            staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap(),
        ),
    )
    .unwrap();
    let narrowed_scope = vec!["README.md".to_string()];
    let mut narrowed_record = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        staged_visible_tree_oid(&root, &config.agent, &narrowed_scope).unwrap(),
    );
    narrowed_record.scope = narrowed_scope.clone();
    append_history_record(&root, &expectation, &narrowed_record).unwrap();

    set_non_selected_expectation_scopes_to_full(&root, std::slice::from_ref(&expectation)).unwrap();

    let records = read_history_records(&root, &expectation).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].scope, full_scope());
    assert_eq!(
        same_tree_history_record(&root, &config.agent, &expectation)
            .unwrap()
            .map(|record| record.scope),
        Some(narrowed_scope)
    );
    let mut history_cache = HistoryCache::new();
    assert_eq!(
        latest_stored_q_scope_with_cache(&root, &config.agent, &expectation, &mut history_cache,)
            .unwrap(),
        Some(full_scope())
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lazy_full_scope_reset_does_not_create_missing_history_files() {
    let root = git_project("check-lazy-reset-missing-history");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &[], false, true).selected[0].clone();
    let path = history_path(&root, &expectation).unwrap();

    set_non_selected_expectation_scopes_to_full(&root, &[expectation]).unwrap();

    assert!(!path.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lazy_full_scope_reset_preserves_full_scope_cooldown_pass() {
    let root = git_project("check-lazy-reset-full-cooldown");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#,
    )
    .unwrap();
    let expectation = check_options(&config, &[], false, true).selected[0].clone();
    let mut record = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap(),
    );
    record.timestamp = format_record_timestamp(unix_timestamp().unwrap());
    append_history_record(&root, &expectation, &record).unwrap();
    let reset_history_path = history_path(&root, &expectation).unwrap();

    set_non_selected_expectation_scopes_to_full(&root, std::slice::from_ref(&expectation)).unwrap();

    assert!(reset_history_path.exists());
    assert_eq!(read_history_records(&root, &expectation).unwrap().len(), 1);
    let mut history_cache = HistoryCache::new();
    assert!(cooldown_history_record(
        &root,
        &config.agent,
        &expectation,
        &mut history_cache,
        unix_timestamp().unwrap(),
    )
    .unwrap()
    .is_some());
    let _ = fs::remove_dir_all(root);
}
