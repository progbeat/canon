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
fn check_runner_requires_human_review_for_unparseable_response() {
    let root = git_project("check-unparseable-first-response");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&["not parseable"]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, UNPARSEABLE_OBSERVED);
    assert_eq!(runner.prompts.len(), 1);
    assert!(read_history_records(&root, &options.selected[0])
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_marks_unparseable_after_response_parse_fails() {
    let root = git_project("check-unparseable");
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
    assert_eq!(records.records[0].observed, UNPARSEABLE_OBSERVED);
    assert!(records.records[0].evidence.contains("response: <empty>"));
    assert_eq!(runner.prompts.len(), 1);
    assert!(read_history_records(&root, &options.selected[0])
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_marks_absent_response_scope_unparseable() {
    let root = git_project("check-absent-response-scope");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer(
        "yes",
        "missing.rs would be enough if it existed",
        &["missing.rs"],
    )]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, UNPARSEABLE_OBSERVED);
    assert!(records.records[0].evidence.contains("missing.rs"));
    assert_eq!(records.records[0].scope, vec![".".to_string()]);
    assert!(read_history_records(&root, &options.selected[0])
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_retries_full_scope_for_restricted_idk_non_answer() {
    let root = git_project("check-narrow-idk");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer("idk", "src/main.rs is insufficient", &["src"])]);

    let report =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(report.records[0].observed, OBSERVED_IDK);
    assert_eq!(report.narrowing.attempted, 0);
    assert_eq!(runner.start_scopes, vec![vec![".".to_string()]]);
    assert!(read_history_records(&root, &options.selected[0])
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_does_not_retry_unparseable_response() {
    let root = git_project("check-unparseable-no-retry");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let later_answer = answer("yes", "README.md", &["."]);
    let mut runner = FakeRunner::new(&["not json", &later_answer]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, UNPARSEABLE_OBSERVED);
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_requires_human_review_when_evidence_stays_empty() {
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
    assert!(!records.records[0].passed());
    assert!(record_requires_human_review(&records.records[0]));
    assert_eq!(records.records[0].observed, EMPTY_EVIDENCE_OBSERVED);
    assert!(records.records[0].evidence.contains("evidence was empty"));
    assert_eq!(runner.prompts.len(), 1);
    assert!(read_history_records(&root, &options.selected[0])
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_requires_human_review_for_malformed_answer() {
    let root = git_project("check-malformed-answer");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let malformed = answer("malformed", "question is malformed", &["."]);
    let mut runner = FakeRunner::new(&[&malformed]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, "malformed");
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_does_not_retry_after_malformed_empty_evidence() {
    let root = git_project("check-malformed-empty-evidence");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let malformed = answer("malformed", "", &["."]);
    let mut runner = FakeRunner::new(&[&malformed, &malformed, &answer("yes", "late", &["."])]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, EMPTY_EVIDENCE_OBSERVED);
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_does_not_retry_after_empty_evidence() {
    let root = git_project("check-empty-evidence-no-retry");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let later_answer = answer("yes", "README.md has evidence", &["."]);
    let mut runner = FakeRunner::new(&[&answer("yes", "", &["."]), &later_answer]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, EMPTY_EVIDENCE_OBSERVED);
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_keeps_semantic_malformed_as_human_review_failure() {
    let root = git_project("check-full-malformed");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer(
        "malformed",
        "full scope response stayed malformed",
        &["."],
    )]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, "malformed");
    assert_eq!(runner.start_scopes, vec![vec![".".to_string()]]);
    let _ = fs::remove_dir_all(root);
}
