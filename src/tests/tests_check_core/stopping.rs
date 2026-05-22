use crate::check::run_check_with_runner;
use crate::check_selection::parse_check_options;
use crate::tests::{
    answer, check_config_yaml, check_options, git_project, parse_check_config, test_selector,
    FakeRunner,
};
use crate::token_usage_types::{ContextCompactionEvent, EvaluatorTurnUsage, TokenUsage};
use serde_json::json;
use std::fs;

#[test]
fn check_runner_stops_after_first_failure_by_default() {
    let root = git_project("check-default-stop");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = parse_check_options(
        &config,
        &[
            test_selector(&config, "1").into(),
            test_selector(&config, "2").into(),
        ],
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[&answer("no", "wrong", &["."])]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(records.records.len(), 1);
    assert!(!records.records[0].passed());
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_all_checks_full_selected_set_after_failure() {
    let root = git_project("check-all-after-failure");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = parse_check_options(
        &config,
        &[
            "--all".into(),
            test_selector(&config, "1").into(),
            test_selector(&config, "2").into(),
        ],
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[
        &answer("no", "wrong", &["."]),
        &answer("no", "second answer", &["."]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(records.records.len(), 2);
    assert_eq!(runner.prompts.len(), 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_breaks_after_turn_token_limit() {
    let root = git_project("check-break-after-tokens");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let mut options = parse_check_options(
        &config,
        &[
            "--all".into(),
            "--break-after-tokens".into(),
            "100".into(),
            test_selector(&config, "1").into(),
            test_selector(&config, "2").into(),
        ],
    )
    .unwrap();
    options.ignore_cache = true;
    let mut runner = FakeRunner::new(&[
        &answer("yes", "first answer", &["."]),
        &answer("no", "second answer", &["."]),
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
        thread_id: "thread-1".to_string(),
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
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_breaks_after_context_compaction_after_recording_turn() {
    let root = git_project("check-break-after-compaction");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let mut options = check_options(&config, &["1", "2"], true, true);
    options.ignore_cache = true;
    let mut runner = FakeRunner::new(&[
        &answer("yes", "first answer", &["."]),
        &answer("no", "second answer", &["."]),
    ]);
    runner.turn_usages.push_back(Some(EvaluatorTurnUsage {
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        usage: TokenUsage {
            total_tokens: 10,
            input_tokens: 7,
            cached_input_tokens: 0,
            output_tokens: 3,
            reasoning_output_tokens: 0,
        },
        token_usage_updates: Vec::new(),
        context_compaction_events: vec![ContextCompactionEvent {
            sequence: 1,
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            method: "item/completed".to_string(),
            event: json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "item": {"type": "contextCompaction"}
                }
            }),
        }],
    }));

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(records.records.len(), 1);
    assert_eq!(records.records[0].id, options.selected[0].id);
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}
