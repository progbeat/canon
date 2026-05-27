use crate::check::run_check_with_runner;
use crate::check_order_state::latest_recorded_non_pass_timestamp;
use crate::check_output::record_requires_human_review;
use crate::check_selection::parse_check_options;
use crate::history::read_history_records;
use crate::tests::{
    answer, check_config_yaml, check_options, error_response, git_project, parse_check_config,
    test_selector, FakeRunner,
};
use crate::token_usage_types::{EvaluatorTurnUsage, TokenUsage};
use crate::ERROR_INSUFFICIENT_EVIDENCE;
use std::fs;

#[test]
fn check_runner_verifies_narrowed_scope_before_history_reuse() {
    let root = git_project("check-narrowing-accepted");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "full scope supports it", &["src/main.rs"]),
        &answer("yes", "src/main.rs still supports it", &["src/main.rs"]),
    ]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(records.records[0].passed());
    assert_eq!(records.records[0].scope, vec!["src/main.rs".to_string()]);
    assert_eq!(
        runner.start_scopes,
        vec![vec![".".to_string()], vec!["src/main.rs".to_string()]]
    );
    assert_eq!(
        read_history_records(&root, &options.selected[0])
            .unwrap()
            .len(),
        1
    );
    let _ = fs::remove_dir_all(root);

    let root = git_project("check-narrowing-accepted-incorrect");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("no", "full scope fails it", &["src/main.rs"]),
        &answer("no", "src/main.rs still fails it", &["src/main.rs"]),
    ]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, "no");
    assert_eq!(records.records[0].scope, vec!["src/main.rs".to_string()]);
    assert_eq!(
        read_history_records(&root, &options.selected[0])
            .unwrap()
            .len(),
        1
    );
    let _ = fs::remove_dir_all(root);

    let root = git_project("check-narrowing-accepted-changed-incorrect");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "full scope supports it", &["src/main.rs"]),
        &answer(
            "no",
            "src/main.rs changes to a failing answer",
            &["src/main.rs"],
        ),
    ]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, "no");
    assert_eq!(
        records.records[0].evidence,
        "`src/main.rs`: src/main.rs changes to a failing answer"
    );
    assert_eq!(records.records[0].scope, vec!["src/main.rs".to_string()]);
    let history = read_history_records(&root, &options.selected[0]).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].observed, "no");
    assert_eq!(
        history[0].evidence,
        "`src/main.rs`: src/main.rs changes to a failing answer"
    );
    assert_eq!(history[0].scope, vec!["src/main.rs".to_string()]);
    let _ = fs::remove_dir_all(root);

    let root = git_project("check-narrowing-rejected");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("no", "full scope fails it", &["src/main.rs"]),
        &answer(
            "yes",
            "src/main.rs changes to a passing answer",
            &["src/main.rs"],
        ),
    ]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(records.records[0].passed());
    assert_eq!(records.records[0].observed, "yes");
    assert_eq!(
        records.records[0].evidence,
        "`src/main.rs`: src/main.rs changes to a passing answer"
    );
    assert_eq!(records.records[0].scope, vec!["src/main.rs"]);
    let history = read_history_records(&root, &options.selected[0]).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].observed, "yes");
    assert_eq!(
        history[0].evidence,
        "`src/main.rs`: src/main.rs changes to a passing answer"
    );
    assert_eq!(history[0].scope, vec!["src/main.rs"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_treats_full_scope_insufficient_evidence_as_human_review() {
    let root = git_project("check-full-scope-insufficient-evidence");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: Answer from files only.
  ignore:
    - "target/**"
  plugins: []
expectations:
  - q: "Are there any unused files?"
    a: "no"
"#,
    )
    .unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&error_response(
        ERROR_INSUFFICIENT_EVIDENCE,
        "not enough evidence",
    )]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert!(record_requires_human_review(&records.records[0]));
    assert_eq!(records.records[0].observed, ERROR_INSUFFICIENT_EVIDENCE);
    assert!(
        latest_recorded_non_pass_timestamp(&root, &options.selected[0])
            .unwrap()
            .is_some()
    );
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_rejects_narrowing_verification_error() {
    let root = git_project("check-narrowing-verification-error");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "full scope supports it", &["src/main.rs"]),
        &error_response(
            ERROR_INSUFFICIENT_EVIDENCE,
            "src/main.rs alone is insufficient",
        ),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records[0].passed());
    assert_eq!(
        records.records[0].evidence,
        "`src/main.rs`: full scope supports it"
    );
    assert_eq!(records.records[0].scope, vec![".".to_string()]);
    assert_eq!(records.narrowing.attempted, 1);
    assert_eq!(records.narrowing.accepted, 0);
    assert_eq!(records.narrowing.rejected, 1);
    assert_eq!(
        runner.start_scopes,
        vec![vec![".".to_string()], vec!["src/main.rs".to_string()]]
    );
    assert_eq!(runner.prompts.len(), 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_verifies_narrowed_scope_before_token_break_stop() {
    let root = git_project("check-narrowing-before-token-break");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let mut options = parse_check_options(
        &config,
        &[
            "--break-after-tokens".into(),
            "100".into(),
            test_selector(&config, "1").into(),
            test_selector(&config, "2").into(),
        ],
    )
    .unwrap();
    options.ignore_cache = true;
    let mut runner = FakeRunner::new(&[
        &answer("yes", "full scope supports it", &["src/main.rs"]),
        &answer("yes", "src/main.rs still supports it", &["src/main.rs"]),
        &answer("no", "second answer should not run", &["."]),
    ]);
    runner.turn_usages.push_back(Some(EvaluatorTurnUsage {
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        usage: TokenUsage {
            total_tokens: 121,
            input_tokens: 90,
            cached_input_tokens: 10,
            output_tokens: 11,
            reasoning_output_tokens: 0,
        },
        token_usage_updates: Vec::new(),
        context_compaction_events: Vec::new(),
    }));
    runner.turn_usages.push_back(Some(EvaluatorTurnUsage {
        thread_id: "thread-2".to_string(),
        turn_id: "turn-2".to_string(),
        usage: TokenUsage {
            total_tokens: 12,
            input_tokens: 10,
            cached_input_tokens: 0,
            output_tokens: 2,
            reasoning_output_tokens: 0,
        },
        token_usage_updates: Vec::new(),
        context_compaction_events: Vec::new(),
    }));

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(records.records.len(), 1);
    assert!(records.records[0].passed());
    assert_eq!(records.records[0].scope, vec!["src/main.rs".to_string()]);
    assert_eq!(records.narrowing.attempted, 1);
    assert_eq!(records.narrowing.accepted, 1);
    assert_eq!(
        runner.start_scopes,
        vec![vec![".".to_string()], vec!["src/main.rs".to_string()]]
    );
    assert_eq!(runner.prompts.len(), 2);
    let _ = fs::remove_dir_all(root);
}
