use super::*;

#[test]
fn failed_evaluator_turn_writes_response_log_with_usage() {
    let root = git_project("failed-turn-response-log");
    enable_diagnostic_logs(&root);
    let mut runner = FakeRunner::new_results(vec![Err(EvaluatorError::failure(
        EvaluatorFailureKind::ContextWindow,
        "context window exceeded",
    ))]);
    runner.turn_usages.push_back(Some(EvaluatorTurnUsage {
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        usage: TokenUsage {
            total_tokens: 10,
            input_tokens: 7,
            cached_input_tokens: 2,
            output_tokens: 3,
            reasoning_output_tokens: 1,
        },
        token_usage_updates: vec![TokenUsageUpdate {
            sequence: 1,
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            token_usage: json!({
                "last": {
                    "totalTokens": 10,
                    "inputTokens": 7,
                    "cachedInputTokens": 2,
                    "outputTokens": 3,
                    "reasoningOutputTokens": 1
                }
            }),
            last_usage: TokenUsage {
                total_tokens: 10,
                input_tokens: 7,
                cached_input_tokens: 2,
                output_tokens: 3,
                reasoning_output_tokens: 1,
            },
        }],
        context_compaction_events: Vec::new(),
    }));
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();
    let turn = EvaluatorTurnContext {
        session_id: "session-1",
        model: None,
        thinking: "low",
    };
    let mut diagnostic_log_ref = Some(&mut diagnostic_log);

    let err = match ask_and_log(
        &mut runner,
        &turn,
        "Question?",
        &mut diagnostic_log_ref,
        Some("id-1"),
        1,
        "initial",
    ) {
        Ok(_) => panic!("expected evaluator failure"),
        Err(err) => err,
    };

    assert_eq!(err.kind(), Some(EvaluatorFailureKind::ContextWindow));
    let log = fs::read_to_string(diagnostic_log.path()).unwrap();
    let response = log
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["event"] == "agent.response")
        .unwrap();
    assert_eq!(response["level"].as_str(), Some("error"));
    assert_eq!(response["error"].as_str(), Some("context window exceeded"));
    assert_eq!(response["threadId"].as_str(), Some("thread-1"));
    assert_eq!(response["turnId"].as_str(), Some("turn-1"));
    assert!(response.get("tokenUsage").is_none());
    assert_eq!(response["tokenUsageUpdates"][0]["sequence"], json!(1));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn evaluator_turn_log_writes_aggregate_usage_when_raw_updates_are_absent() {
    let root = git_project("turn-response-log-aggregate-usage");
    enable_diagnostic_logs(&root);
    let mut runner = FakeRunner::new(&[&answer("yes", "evidence", &["."])]);
    runner.turn_usages.push_back(Some(EvaluatorTurnUsage {
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        usage: TokenUsage {
            total_tokens: 10,
            input_tokens: 7,
            cached_input_tokens: 2,
            output_tokens: 3,
            reasoning_output_tokens: 1,
        },
        token_usage_updates: Vec::new(),
        context_compaction_events: Vec::new(),
    }));
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();
    let turn = EvaluatorTurnContext {
        session_id: "session-1",
        model: None,
        thinking: "low",
    };
    let mut diagnostic_log_ref = Some(&mut diagnostic_log);

    ask_and_log(
        &mut runner,
        &turn,
        "Question?",
        &mut diagnostic_log_ref,
        Some("id-1"),
        1,
        "initial",
    )
    .unwrap();

    let log = fs::read_to_string(diagnostic_log.path()).unwrap();
    let response = log
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["event"] == "agent.response")
        .unwrap();
    assert_eq!(response["threadId"].as_str(), Some("thread-1"));
    assert_eq!(response["turnId"].as_str(), Some("turn-1"));
    assert!(response.get("tokenUsageUpdates").is_none());
    assert_eq!(response["tokenUsage"]["totalTokens"], json!(10));
    assert_eq!(response["tokenUsage"]["inputTokens"], json!(7));
    assert_eq!(response["tokenUsage"]["cachedInputTokens"], json!(2));
    assert_eq!(response["tokenUsage"]["outputTokens"], json!(3));
    assert_eq!(response["tokenUsage"]["reasoningOutputTokens"], json!(1));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn evaluator_turn_log_errors_when_successful_response_lacks_usage() {
    let root = git_project("turn-response-log-missing-usage");
    enable_diagnostic_logs(&root);
    let mut runner = FakeRunner::new(&[&answer("yes", "evidence", &["."])]);
    runner.turn_usages.push_back(None);
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();
    let turn = EvaluatorTurnContext {
        session_id: "session-1",
        model: None,
        thinking: "low",
    };
    let mut diagnostic_log_ref = Some(&mut diagnostic_log);

    let err = match ask_and_log(
        &mut runner,
        &turn,
        "Question?",
        &mut diagnostic_log_ref,
        Some("id-1"),
        1,
        "initial",
    ) {
        Ok(_) => panic!("expected missing usage error"),
        Err(err) => err,
    };

    assert_eq!(err.message_str(), "missing evaluator turn usage");
    let log = fs::read_to_string(diagnostic_log.path()).unwrap();
    let response = log
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["event"] == "agent.turn_error")
        .unwrap();
    assert_eq!(response["level"].as_str(), Some("error"));
    assert_eq!(
        response["error"].as_str(),
        Some("missing evaluator turn usage")
    );
    assert_eq!(
        response["response"]["sessionId"].as_str(),
        Some("session-1")
    );
    assert_eq!(response["threadId"].as_str(), Some("session-1"));
    assert_eq!(response["turnId"].as_str(), Some("<missing>"));
    assert_eq!(response["tokenUsage"]["totalTokens"], json!(0));
    assert_eq!(response["tokenUsage"]["inputTokens"], json!(0));
    assert_eq!(response["tokenUsage"]["cachedInputTokens"], json!(0));
    assert_eq!(response["tokenUsage"]["outputTokens"], json!(0));
    assert_eq!(response["tokenUsage"]["reasoningOutputTokens"], json!(0));
    assert_eq!(response["tokenUsageUnavailable"], json!(true));
    assert!(response.get("tokenUsageUpdates").is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_requires_human_review_for_unparsable_response() {
    let root = git_project("check-unparsable-first-response");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&["not parseable"]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, ERROR_UNPARSABLE);
    assert_eq!(runner.prompts.len(), 1);
    assert!(read_history_records(&root, &options.selected[0])
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_marks_unparsable_after_response_parse_fails() {
    let root = git_project("check-unparsable");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[""]);
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();
    let records = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        Some(&mut diagnostic_log),
        None,
    )
    .unwrap();
    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, ERROR_UNPARSABLE);
    assert!(records.records[0].evidence.contains("response: <empty>"));
    assert_eq!(runner.prompts.len(), 1);
    assert!(read_history_records(&root, &options.selected[0])
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_ignores_invalid_q_scope_suggestion() {
    let root = git_project("check-invalid-q-scope-suggestion");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let response = serde_json::to_string(&json!({
        "answer": "yes",
        "evidence": "`README.md`: full scope supports yes",
        "qScopeSuggestion": ["../missing.rs"]
    }))
    .unwrap();
    let mut runner = FakeRunner::new(&[&response]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records[0].passed());
    assert_eq!(records.records[0].observed, "yes");
    assert_eq!(records.records[0].scope, vec![".".to_string()]);
    assert_eq!(records.records[0].suggested_q_scope, None);
    assert_eq!(
        read_history_records(&root, &options.selected[0])
            .unwrap()
            .len(),
        1
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_records_full_scope_insufficient_evidence_error() {
    let root = git_project("check-full-scope-insufficient-evidence-error");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&error_response(
        ERROR_INSUFFICIENT_EVIDENCE,
        "src/main.rs is insufficient",
    )]);

    let report =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(report.records[0].observed, ERROR_INSUFFICIENT_EVIDENCE);
    assert_eq!(report.narrowing.attempted, 0);
    assert_eq!(runner.start_scopes, vec![vec![".".to_string()]]);
    assert!(read_history_records(&root, &options.selected[0])
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_does_not_retry_unparsable_response() {
    let root = git_project("check-unparsable-no-retry");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let later_answer = answer("yes", "README.md", &["."]);
    let mut runner = FakeRunner::new(&["not json", &later_answer]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, ERROR_UNPARSABLE);
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_accepts_empty_evidence_answer() {
    let root = git_project("check-empty-evidence");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer("yes", "", &["."])]);
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();
    let records = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        Some(&mut diagnostic_log),
        None,
    )
    .unwrap();
    assert!(records.records[0].passed());
    assert!(!record_requires_human_review(&records.records[0]));
    assert_eq!(records.records[0].observed, "yes");
    assert_eq!(records.records[0].evidence, "");
    assert_eq!(runner.prompts.len(), 1);
    assert_eq!(
        read_history_records(&root, &options.selected[0])
            .unwrap()
            .len(),
        1
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_requires_human_review_when_evidence_has_no_project_citation() {
    let root = git_project("check-missing-evidence-citation");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let response = serde_json::to_string(&json!({
        "answer": "yes",
        "evidence": "README.md has evidence but is not cited",
        "qScopeSuggestion": ["."]
    }))
    .unwrap();
    let mut runner = FakeRunner::new(&[&response]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records[0].passed());
    assert!(!record_requires_human_review(&records.records[0]));
    assert_eq!(records.records[0].observed, "yes");
    assert_eq!(
        read_history_records(&root, &options.selected[0])
            .unwrap()
            .len(),
        1
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_allows_self_contained_arithmetic_without_project_citation() {
    let root = git_project("check-self-contained-arithmetic");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: Answer directly.
  ignore: []
  plugins: []
expectations:
  - q: "2+2=?"
    a: "4"
"#,
    )
    .unwrap();
    let options = check_options(&config, &["1"], false, true);
    let response = serde_json::to_string(&json!({
        "answer": "4",
        "evidence": "Derived directly from the user prompt.",
        "qScopeSuggestion": ["<none>"]
    }))
    .unwrap();
    let mut runner = FakeRunner::new(&[&response]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records[0].passed());
    assert_eq!(records.records[0].observed, "4");
    assert_eq!(
        records.records[0].evidence,
        "Derived directly from the user prompt."
    );
    assert_eq!(records.records[0].scope, full_scope());
    assert_eq!(
        read_history_records(&root, &options.selected[0])
            .unwrap()
            .len(),
        1
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_records_mismatched_answer_without_project_citation() {
    let root = git_project("check-invalid-answer-missing-evidence-citation");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let response = serde_json::to_string(&json!({
        "answer": "maybe",
        "evidence": "README.md might have evidence but is not cited",
        "qScopeSuggestion": ["."]
    }))
    .unwrap();
    let mut runner = FakeRunner::new(&[&response]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert!(!record_requires_human_review(&records.records[0]));
    assert_eq!(records.records[0].observed, "maybe");
    assert_eq!(
        read_history_records(&root, &options.selected[0])
            .unwrap()
            .len(),
        1
    );
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_records_second_mismatched_answer_without_project_citation() {
    let root = git_project("check-second-invalid-answer-missing-evidence-citation");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let response = serde_json::to_string(&json!({
        "answer": "unclear",
        "evidence": "question cannot be answered but no project citation is present",
        "qScopeSuggestion": ["."]
    }))
    .unwrap();
    let mut runner = FakeRunner::new(&[&response]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert!(!record_requires_human_review(&records.records[0]));
    assert_eq!(records.records[0].observed, "unclear");
    assert_eq!(
        read_history_records(&root, &options.selected[0])
            .unwrap()
            .len(),
        1
    );
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_records_mismatched_yes_no_answer() {
    let root = git_project("check-mismatched-yes-no-answer");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let invalid = answer("unclear", "question needs a yes/no answer", &["."]);
    let mut runner = FakeRunner::new(&[&invalid]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(!records.records[0].passed());
    assert!(!record_requires_human_review(&records.records[0]));
    assert_eq!(records.records[0].observed, "unclear");
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_does_not_retry_after_mismatched_answer() {
    let root = git_project("check-mismatched-answer-no-retry");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let invalid = answer("unclear", "question needs a clearer answer", &["."]);
    let mut runner = FakeRunner::new(&[&invalid, &invalid, &answer("yes", "late", &["."])]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert!(!record_requires_human_review(&records.records[0]));
    assert_eq!(records.records[0].observed, "unclear");
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_does_not_retry_after_empty_evidence_answer() {
    let root = git_project("check-empty-evidence-no-retry");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let later_answer = answer("yes", "README.md has evidence", &["."]);
    let mut runner = FakeRunner::new(&[&answer("yes", "", &["."]), &later_answer]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records[0].passed());
    assert_eq!(records.records[0].observed, "yes");
    assert!(!record_requires_human_review(&records.records[0]));
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_keeps_mismatched_answer_as_failure() {
    let root = git_project("check-full-mismatched-answer");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer(
        "unclear",
        "full scope response is a schema-valid answer that does not match expected",
        &["."],
    )]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert!(!record_requires_human_review(&records.records[0]));
    assert_eq!(records.records[0].observed, "unclear");
    assert_eq!(runner.start_scopes, vec![vec![".".to_string()]]);
    let _ = fs::remove_dir_all(root);
}
