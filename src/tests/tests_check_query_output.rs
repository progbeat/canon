use super::*;

#[test]
fn check_runner_flushes_each_result_output_record() {
    let root = git_project("check-output");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "README.md says enough", &["."]),
        &answer("no", "README.md says enough", &["."]),
    ]);
    let mut output = FlushCountingWriter::new();
    let records = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        None,
        Some(&mut output),
    )
    .unwrap();
    assert_eq!(records.records.len(), 2);
    assert_eq!(output.flushes, 2);
    let lines = String::from_utf8(output.bytes).unwrap();
    assert_eq!(lines.lines().count(), 2);
    assert!(lines.contains(&format!("{}. OK", options.selected[0].display_id)));
    assert!(lines.contains(&format!("{}. OK", options.selected[1].display_id)));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_output_escapes_non_ascii_control_characters() {
    assert_eq!(
        escape_check_output_text("line\nx\u{1f}y\u{7f}z\u{85}a\u{2028}b\u{2029}c"),
        "line\\nx\\u001fy\\u007fz\\u0085a\\u2028b\\u2029c"
    );
}

#[test]
fn check_output_failed_and_error_records_use_specified_line_counts() {
    let mut record = CheckRecord {
        timestamp: "1970-01-01T00:00:00Z".to_string(),
        id: "AAAAAAAAAAAAAAAAAAAA".to_string(),
        display_id: "A".to_string(),
        number: 7,
        result: CheckResult::Fail,
        prompt: Some("Question?".to_string()),
        expected: Some("yes".to_string()),
        observed: "no".to_string(),
        evidence: "evidence".to_string(),
        scope: vec!["src".to_string()],
        visible_tree_oid: "hash".to_string(),
        cache_key: None,
    };

    assert_eq!(render_check_output_record(&record).lines().count(), 6);

    record.observed = OBSERVED_IDK.to_string();
    assert_eq!(render_check_output_record(&record).lines().count(), 5);
}

#[test]
fn check_summary_counts_cached_passes_as_skipped() {
    let report = CheckRunReport {
        records: Vec::new(),
        non_selected: Vec::new(),
        cached: Vec::new(),
        evaluated: 0,
        selected: 0,
        skipped: 1,
        silent: 2,
        narrowing: NarrowingStats::default(),
    };

    let summary = render_check_summary(&report, Duration::from_secs(1));

    assert!(summary.contains("1 skipped"));
    assert!(!summary.contains("passed"));
}

#[test]
fn pass_improvement_notice_uses_specified_pluralization() {
    assert_eq!(pass_improvement_notice(0), None);
    assert_eq!(
        pass_improvement_notice(1).as_deref(),
        Some("▷ +1 pass compared to HEAD. Commit the staged changes NOW!")
    );
    assert_eq!(
        pass_improvement_notice(2).as_deref(),
        Some("▷ +2 passes compared to HEAD. Commit the staged changes NOW!")
    );
}

#[test]
fn check_agent_message_uses_required_all_pass_and_fallback_text() {
    let root = git_project("check-agent-message");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let pass_report = CheckRunReport {
        records: Vec::new(),
        non_selected: Vec::new(),
        cached: Vec::new(),
        evaluated: 0,
        selected: 0,
        skipped: 0,
        silent: 0,
        narrowing: NarrowingStats::default(),
    };
    assert_eq!(
        check_agent_message(
            &root,
            &config,
            &pass_report,
            &mut HistoryCache::new(),
            &mut VisibleTreeOidCache::new(),
        )
        .unwrap(),
        "✓ All checks passed. Commit is allowed."
    );

    let fail_report = CheckRunReport {
        records: vec![sample_record(1, "fail")],
        non_selected: Vec::new(),
        cached: Vec::new(),
        evaluated: 1,
        selected: 1,
        skipped: 0,
        silent: 0,
        narrowing: NarrowingStats::default(),
    };
    assert_eq!(
        check_agent_message(
            &root,
            &config,
            &fail_report,
            &mut HistoryCache::new(),
            &mut VisibleTreeOidCache::new(),
        )
        .unwrap(),
        "▷ Fix the issues and run `canon check` again!"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_agent_message_counts_human_review_as_non_ok() {
    let root = git_project("check-agent-message-human-review");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let mut error_record = sample_record(1, "fail");
    error_record.observed = OBSERVED_IDK.to_string();
    let error_report = CheckRunReport {
        records: vec![error_record],
        non_selected: Vec::new(),
        cached: Vec::new(),
        evaluated: 1,
        selected: 1,
        skipped: 0,
        silent: 0,
        narrowing: NarrowingStats::default(),
    };

    assert_eq!(
        check_agent_message(
            &root,
            &config,
            &error_report,
            &mut HistoryCache::new(),
            &mut VisibleTreeOidCache::new(),
        )
        .unwrap(),
        "▷ Fix the issues and run `canon check` again!"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_pass_notice_counts_missing_head_cache_as_fix() {
    let root = git_project("check-head-pass-notice");
    commit_all(&root, "initial");
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer("yes", "current staged pass", &["."])]);

    let report =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(
        staged_passes_failed_at_head_count(&root, &config.agent, &report).unwrap(),
        1
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_pass_notice_counts_passes_failed_at_head() {
    let root = git_project("check-head-pass-notice-head-fail");
    write_check_config(&root);
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "initial");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let head_hash = gate_head_tree_fingerprint(&root, &config.agent, &full_scope())
        .unwrap()
        .unwrap();
    append_history_record(
        &root,
        &options.selected[0],
        &expectation_record(&config.agent, &options.selected[0], "fail", "no", head_hash),
    )
    .unwrap();
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    let mut runner = FakeRunner::new(&[&answer("yes", "current staged pass", &["."])]);

    let report =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(
        staged_passes_failed_at_head_count(&root, &config.agent, &report).unwrap(),
        1
    );
    assert_eq!(
        staged_pass_notice_count(
            &root,
            &config.agent,
            &report,
            &mut HistoryCache::new(),
            &mut VisibleTreeOidCache::new(),
        )
        .unwrap(),
        1
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_pass_notice_counts_passes_even_with_existing_failure() {
    let root = git_project("check-head-pass-notice-existing-fail");
    write_check_config(&root);
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "initial");
    let config = parse_check_config(check_config_yaml()).unwrap();
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
    let current_visible_tree_oid =
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap();
    let current_pass = expectation_record(
        &config.agent,
        &options.selected[0],
        "pass",
        "yes",
        current_visible_tree_oid.clone(),
    );
    let current_fail = expectation_record(
        &config.agent,
        &options.selected[1],
        "fail",
        "yes",
        current_visible_tree_oid,
    );
    append_history_record(&root, &options.selected[0], &current_pass).unwrap();
    append_history_record(&root, &options.selected[1], &current_fail).unwrap();
    let report = CheckRunReport {
        records: vec![current_pass, current_fail],
        non_selected: Vec::new(),
        cached: Vec::new(),
        evaluated: 2,
        selected: 2,
        skipped: 0,
        silent: 0,
        narrowing: NarrowingStats::default(),
    };

    assert_eq!(
        staged_pass_notice_count(
            &root,
            &config.agent,
            &report,
            &mut HistoryCache::new(),
            &mut VisibleTreeOidCache::new(),
        )
        .unwrap(),
        1
    );
    assert_eq!(
        check_agent_messages(
            &root,
            &config,
            &report,
            &mut HistoryCache::new(),
            &mut VisibleTreeOidCache::new(),
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
fn check_agent_message_allows_commit_when_only_regression_cache_is_missing() {
    let root = git_project("check-agent-message-gate-missing-cache");
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
  - q: "Unevaluated?"
    a: "yes"
"#;
    fs::create_dir_all(root.join(".canon")).unwrap();
    fs::write(root.join(CHECK_PATH), yaml).unwrap();
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "initial");
    let config = parse_check_config(yaml).unwrap();
    let options = check_options(&config, &["1", "2", "3"], false, true);
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
            head_hash.clone(),
        ),
    )
    .unwrap();
    append_history_record(
        &root,
        &options.selected[2],
        &expectation_record(
            &config.agent,
            &options.selected[2],
            "pass",
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
    let current_visible_tree_oid =
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap();
    let current_pass = expectation_record(
        &config.agent,
        &options.selected[0],
        "pass",
        "yes",
        current_visible_tree_oid.clone(),
    );
    let current_fail = expectation_record(
        &config.agent,
        &options.selected[1],
        "fail",
        "yes",
        current_visible_tree_oid,
    );
    append_history_record(&root, &options.selected[0], &current_pass).unwrap();
    append_history_record(&root, &options.selected[1], &current_fail).unwrap();
    let report = CheckRunReport {
        records: vec![current_pass, current_fail],
        non_selected: Vec::new(),
        cached: Vec::new(),
        evaluated: 2,
        selected: 3,
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
            &mut VisibleTreeOidCache::new(),
        )
        .unwrap(),
        vec![
            "▷ +1 pass compared to HEAD. Commit the staged changes NOW!".to_string(),
            "▷ Then fix the remaining issues and run `canon check` again!".to_string(),
        ]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_agent_message_prioritizes_regressions_over_fixes() {
    let root = git_project("check-agent-message-regression");
    write_check_config(&root);
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "initial");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let head_hash = gate_head_tree_fingerprint(&root, &config.agent, &full_scope())
        .unwrap()
        .unwrap();
    append_history_record(
        &root,
        &options.selected[1],
        &expectation_record(&config.agent, &options.selected[1], "pass", "no", head_hash),
    )
    .unwrap();
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    let current_visible_tree_oid =
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap();
    let current_fail = expectation_record(
        &config.agent,
        &options.selected[1],
        "fail",
        "yes",
        current_visible_tree_oid.clone(),
    );
    append_history_record(&root, &options.selected[1], &current_fail).unwrap();
    let report = CheckRunReport {
        records: vec![
            expectation_record(
                &config.agent,
                &options.selected[0],
                "pass",
                "yes",
                current_visible_tree_oid.clone(),
            ),
            current_fail,
        ],
        non_selected: Vec::new(),
        cached: Vec::new(),
        evaluated: 2,
        selected: 2,
        skipped: 0,
        silent: 0,
        narrowing: NarrowingStats::default(),
    };

    assert_eq!(
        check_agent_message(
            &root,
            &config,
            &report,
            &mut HistoryCache::new(),
            &mut VisibleTreeOidCache::new(),
        )
        .unwrap(),
        "▷ Fix the issues and run `canon check` again!"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn query_mode_uses_agent_and_does_not_write_history() {
    let root = git_project("query-mode");
    enable_diagnostic_logs(&root);
    let config = parse_check_config(check_config_yaml()).unwrap();
    let mut runner = FakeRunner::new(&[
        &answer("no", "src/main.rs says no", &["src"]),
        &answer("no", "src/main.rs says no", &["src"]),
    ]);
    let token_usage = json!({
        "last": {
            "totalTokens": 10,
            "inputTokens": 7,
            "cachedInputTokens": 3,
            "outputTokens": 3,
            "reasoningOutputTokens": 1
        },
        "total": {
            "totalTokens": 10,
            "inputTokens": 7,
            "cachedInputTokens": 3,
            "outputTokens": 3,
            "reasoningOutputTokens": 1
        },
        "modelContextWindow": 200000
    });
    runner.turn_usages.push_back(Some(EvaluatorTurnUsage {
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        usage: TokenUsage {
            total_tokens: 10,
            input_tokens: 7,
            cached_input_tokens: 3,
            output_tokens: 3,
            reasoning_output_tokens: 1,
        },
        token_usage_updates: vec![TokenUsageUpdate {
            sequence: 1,
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            token_usage: token_usage.clone(),
            last_usage: TokenUsage {
                total_tokens: 10,
                input_tokens: 7,
                cached_input_tokens: 3,
                output_tokens: 3,
                reasoning_output_tokens: 1,
            },
        }],
        context_compaction_events: Vec::new(),
    }));
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();
    let runtime = CheckRuntime {
        root: &root,
        snapshot_root: &root,
        config: &config,
    };
    let mut interrogation_state = InterrogationState::new();

    let result = run_query_with_runner(
        &runtime,
        "Ad-hoc question?",
        None,
        &full_scope(),
        &mut runner,
        Some(&mut diagnostic_log),
        &mut interrogation_state,
    )
    .unwrap();

    assert_eq!(result.answer.answer, "no");
    assert_eq!(result.answer.scope, vec!["src"]);
    assert_eq!(
        runner.prompts,
        vec![
            "Ad-hoc question?".to_string(),
            "Ad-hoc question?".to_string()
        ]
    );
    assert_eq!(
        runner.start_scopes,
        vec![vec![".".to_string()], vec!["src".to_string()]]
    );
    assert_eq!(
        runner.start_models,
        vec![
            Some("gpt-5.4-mini".to_string()),
            Some("gpt-5.4-mini".to_string())
        ]
    );
    assert_eq!(
        runner.start_thinking,
        vec!["medium".to_string(), "medium".to_string()]
    );
    let cache_dir = root.join(".git/canon/cache");
    assert!(!cache_dir.exists() || fs::read_dir(&cache_dir).unwrap().next().is_none());
    let output = render_query_output(&result.answer);
    assert_eq!(
        output,
        "Observed: no\nEvidence: `src/main.rs`: src/main.rs says no\nScope: [\"src\"]\n"
    );
    let log = fs::read_to_string(diagnostic_log.path()).unwrap();
    assert!(log.contains(r#""event":"query.result""#));
    let log_events: Vec<serde_json::Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let request = log_events
        .iter()
        .find(|event| event["event"] == "agent.request")
        .unwrap();
    assert_eq!(request["request"]["sessionId"].as_str(), Some("session-1"));
    assert_eq!(
        request["request"]["prompt"].as_str(),
        Some("Ad-hoc question?")
    );
    assert!(request.get("raw").is_none());
    assert!(request.get("rawRequest").is_none());
    let response = log_events
        .iter()
        .find(|event| event["event"] == "agent.response")
        .unwrap();
    assert!(response["response"]["text"]
        .as_str()
        .unwrap()
        .starts_with(r#"{"answer":"no""#));
    assert!(response.get("raw").is_none());
    assert!(response.get("rawResponse").is_none());
    assert_eq!(response["threadId"].as_str(), Some("thread-1"));
    assert_eq!(response["turnId"].as_str(), Some("turn-1"));
    assert!(response.get("tokenUsage").is_none());
    assert_eq!(response["tokenUsageUpdates"][0]["sequence"], json!(1));
    assert_eq!(
        response["tokenUsageUpdates"][0]["threadId"].as_str(),
        Some("thread-1")
    );
    assert_eq!(
        response["tokenUsageUpdates"][0]["turnId"].as_str(),
        Some("turn-1")
    );
    assert_eq!(response["tokenUsageUpdates"][0]["tokenUsage"], token_usage);
    assert!(!log.contains(r#""event":"expectation.result""#));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn query_mode_accepts_narrowed_incorrect_answer_when_expected_is_known() {
    let root = git_project("query-mode-known-expected-narrowing");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let mut runner = FakeRunner::new(&[
        &answer("yes", "full scope passes", &["src"]),
        &answer("no", "src/main.rs still fails it", &["src"]),
    ]);
    let runtime = CheckRuntime {
        root: &root,
        snapshot_root: &root,
        config: &config,
    };
    let mut interrogation_state = InterrogationState::new();

    let result = run_query_with_runner(
        &runtime,
        "Ad-hoc question?",
        Some("yes"),
        &full_scope(),
        &mut runner,
        None,
        &mut interrogation_state,
    )
    .unwrap();

    assert_eq!(result.answer.answer, "no");
    assert_eq!(
        result.answer.evidence,
        "`src/main.rs`: src/main.rs still fails it"
    );
    assert_eq!(result.answer.scope, vec!["src"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn query_mode_rejects_changed_narrowing_when_expected_is_unknown() {
    let root = git_project("query-mode-unknown-expected-narrowing");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let mut runner = FakeRunner::new(&[
        &answer("yes", "full scope answer", &["src"]),
        &answer("no", "changed narrow answer", &["src"]),
    ]);
    let runtime = CheckRuntime {
        root: &root,
        snapshot_root: &root,
        config: &config,
    };
    let mut interrogation_state = InterrogationState::new();

    let result = run_query_with_runner(
        &runtime,
        "Ad-hoc question?",
        None,
        &full_scope(),
        &mut runner,
        None,
        &mut interrogation_state,
    )
    .unwrap();

    assert_eq!(result.answer.answer, "yes");
    assert_eq!(result.answer.evidence, "`src/main.rs`: full scope answer");
    assert_eq!(result.answer.scope, full_scope());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn query_mode_can_use_explicit_restricted_scope() {
    let root = git_project("query-mode-restricted-scope");
    enable_diagnostic_logs(&root);
    let config = parse_check_config(check_config_yaml()).unwrap();
    let scope = vec!["src".to_string()];
    let mut runner = FakeRunner::new(&[
        &answer("idk", "needs files outside this restricted scope", &["."]),
        &answer("yes", "full-scope evidence answers it", &["."]),
    ]);
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();
    let runtime = CheckRuntime {
        root: &root,
        snapshot_root: &root,
        config: &config,
    };
    let mut interrogation_state = InterrogationState::new();

    let result = run_query_with_runner(
        &runtime,
        "Ad-hoc scoped question?",
        None,
        &scope,
        &mut runner,
        Some(&mut diagnostic_log),
        &mut interrogation_state,
    )
    .unwrap();

    assert_eq!(runner.start_scopes, vec![scope, full_scope()]);
    assert_eq!(
        runner.prompts,
        vec![
            "Ad-hoc scoped question?".to_string(),
            "Ad-hoc scoped question?".to_string()
        ]
    );
    assert_eq!(result.answer.answer, "yes");
    assert_eq!(result.answer.scope, full_scope());
    assert_eq!(
        result.answer.evidence,
        "`README.md`: full-scope evidence answers it"
    );
    assert_eq!(runner.starts, 2);
    let log = fs::read_to_string(diagnostic_log.path()).unwrap();
    assert!(log.contains(r#""event":"query.result""#));
    assert!(log.contains(r#""scope":["."]"#));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn query_mode_errors_when_full_scope_idk_needs_review() {
    let root = git_project("query-mode-full-scope-idk");
    enable_diagnostic_logs(&root);
    let config = parse_check_config(check_config_yaml()).unwrap();
    let mut runner = FakeRunner::new(&[&answer("idk", "full scope still cannot answer", &["."])]);
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();
    let runtime = CheckRuntime {
        root: &root,
        snapshot_root: &root,
        config: &config,
    };
    let mut interrogation_state = InterrogationState::new();

    let err = run_query_with_runner(
        &runtime,
        "Ad-hoc unanswerable question?",
        None,
        &full_scope(),
        &mut runner,
        Some(&mut diagnostic_log),
        &mut interrogation_state,
    )
    .unwrap_err();

    assert!(err.contains("query requires human review: full-scope idk"));
    let log = fs::read_to_string(diagnostic_log.path()).unwrap();
    assert!(!log.contains(r#""event":"query.result""#));
    assert!(log.contains(r#""event":"query.review_required""#));
    assert!(log.contains(r#""reason":"full-scope idk""#));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn query_and_full_scope_expectation_use_identical_first_turn_input() {
    let root = git_project("query-check-first-turn");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let mut query_runner = FakeRunner::new(&[&answer("yes", "query evidence", &["."])]);
    let mut check_runner = FakeRunner::new(&[&answer("yes", "check evidence", &["."])]);
    let runtime = CheckRuntime {
        root: &root,
        snapshot_root: &root,
        config: &config,
    };
    let mut query_state = InterrogationState::new();

    run_query_with_runner(
        &runtime,
        &expectation.q,
        Some(&expectation.a),
        &full_scope(),
        &mut query_runner,
        None,
        &mut query_state,
    )
    .unwrap();
    run_check_with_runner(
        &root,
        &root,
        &config,
        &check_options(&config, &["1"], false, true),
        &mut check_runner,
        None,
        None,
    )
    .unwrap();

    assert_eq!(query_runner.start_scopes, vec![full_scope()]);
    assert_eq!(check_runner.start_scopes, vec![full_scope()]);
    assert_eq!(
        query_runner.start_instructions,
        check_runner.start_instructions
    );
    assert_eq!(query_runner.prompts, check_runner.prompts);
    assert_eq!(query_runner.start_models, check_runner.start_models);
    assert_eq!(query_runner.start_thinking, check_runner.start_thinking);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn successful_narrowing_logs_stats_and_one_final_result() {
    let root = git_project("narrowing-success");
    enable_diagnostic_logs(&root);
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "full answer", &["src"]),
        &answer("yes", "narrow answer", &["src"]),
    ]);
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();

    let report = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        Some(&mut diagnostic_log),
        None,
    )
    .unwrap();

    assert_eq!(report.narrowing.attempted, 1);
    assert_eq!(report.narrowing.accepted, 1);
    assert_eq!(report.narrowing.rejected, 0);
    assert_eq!(report.records[0].scope, vec!["src"]);
    let log = fs::read_to_string(diagnostic_log.path()).unwrap();
    assert_eq!(log.matches(r#""event":"expectation.result""#).count(), 1);
    assert_eq!(log.matches(r#""event":"interrogation.result""#).count(), 2);
    assert!(log.contains(r#""event":"scope.narrowing""#));
    assert!(log.contains(r#""accepted":true"#));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn failed_narrowing_logs_stats_and_keeps_wider_final_result() {
    let root = git_project("narrowing-fail");
    enable_diagnostic_logs(&root);
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let expectation = options.selected[0].clone();
    let mut runner = FakeRunner::new(&[
        &answer("no", "full answer", &["src"]),
        &answer("yes", "narrow answer", &["src"]),
    ]);
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();

    let report = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        Some(&mut diagnostic_log),
        None,
    )
    .unwrap();

    assert_eq!(report.narrowing.attempted, 1);
    assert_eq!(report.narrowing.accepted, 0);
    assert_eq!(report.narrowing.rejected, 1);
    assert_eq!(report.records[0].observed, "no");
    assert_eq!(report.records[0].evidence, "`src/main.rs`: full answer");
    assert_eq!(report.records[0].scope, vec!["."]);
    let history = read_history_records(&root, &expectation).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].observed, "no");
    assert_eq!(history[0].evidence, "`src/main.rs`: full answer");
    assert_eq!(history[0].scope, vec!["."]);
    let log = fs::read_to_string(diagnostic_log.path()).unwrap();
    assert_eq!(log.matches(r#""event":"expectation.result""#).count(), 1);
    assert_eq!(log.matches(r#""event":"interrogation.result""#).count(), 2);
    assert!(log.contains(r#""event":"scope.narrowing""#));
    assert!(log.contains(r#""accepted":false"#));
    let _ = fs::remove_dir_all(root);
}
