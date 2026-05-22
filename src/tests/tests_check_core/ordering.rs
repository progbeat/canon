use crate::check::run_check_with_runner;
use crate::check_order_state::write_latest_non_pass_record;
use crate::check_selection::order_expectations_by_latest_non_pass;
use crate::fs_util::ensure_dir;
use crate::hash::full_scope;
use crate::history::HistoryCache;
use crate::history_append::append_history_record;
use crate::logging::render_runtime_log_event;
use crate::scope_hash::staged_scope_hash;
use crate::tests::{
    answer, check_config_yaml, check_options, expectation_record, git_project, parse_check_config,
    FakeRunner,
};
use crate::{RESULT_FAIL, UNPARSEABLE_OBSERVED};
use serde_json::json;
use std::fs;

#[test]
fn selected_expectations_run_latest_non_pass_first() {
    let root = git_project("check-order-latest-non-pass");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let first = options.selected[0].clone();
    let second = options.selected[1].clone();
    let mut record = expectation_record(
        &config.agent,
        &second,
        "fail",
        "no",
        staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
    );
    record.timestamp = "2026-01-01T00:00:00Z".to_string();
    append_history_record(&root, &second, &record).unwrap();
    let mut history_cache = HistoryCache::new();

    let ordered = order_expectations_by_latest_non_pass(
        &root,
        vec![first, second.clone()],
        &mut history_cache,
    )
    .unwrap();

    assert_eq!(ordered[0].id, second.id);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_orders_latest_non_pass_before_selected_order() {
    let root = git_project("check-order-over-selected-order");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let second = options.selected[1].clone();
    let mut record = expectation_record(
        &config.agent,
        &second,
        "fail",
        "yes",
        staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
    );
    record.timestamp = "2026-01-01T00:00:00Z".to_string();
    append_history_record(&root, &second, &record).unwrap();
    let mut runner = FakeRunner::new(&[
        &answer("no", "second answer", &["."]),
        &answer("yes", "first answer", &["."]),
    ]);

    let report =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(report.records.len(), 2);
    assert_eq!(
        runner.prompts,
        vec!["Second?".to_string(), "First?".to_string()]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn selected_expectations_use_recorded_errors_for_order() {
    let root = git_project("check-order-runtime-errors");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let first = options.selected[0].clone();
    let second = options.selected[1].clone();
    let mut record = expectation_record(
        &config.agent,
        &second,
        "fail",
        UNPARSEABLE_OBSERVED,
        staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
    );
    record.timestamp = "2026-01-01T00:00:00Z".to_string();
    write_latest_non_pass_record(&root, &second, &record).unwrap();
    let mut history_cache = HistoryCache::new();

    let ordered = order_expectations_by_latest_non_pass(
        &root,
        vec![first, second.clone()],
        &mut history_cache,
    )
    .unwrap();

    assert_eq!(ordered[0].id, second.id);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn selected_expectations_ignore_runtime_log_errors_for_order() {
    let root = git_project("check-order-ignores-runtime-errors");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let first = options.selected[0].clone();
    let second = options.selected[1].clone();
    let log_dir = root.join(".git/canon/logs");
    ensure_dir(&log_dir).unwrap();
    let line = render_runtime_log_event(
        "info",
        "expectation.result",
        &[
            ("id", json!(second.id.clone())),
            ("result", json!(RESULT_FAIL)),
            ("observed", json!(UNPARSEABLE_OBSERVED)),
        ],
    )
    .unwrap();
    fs::write(log_dir.join("0.jsonl"), line).unwrap();
    let mut history_cache = HistoryCache::new();

    let ordered = order_expectations_by_latest_non_pass(
        &root,
        vec![first.clone(), second],
        &mut history_cache,
    )
    .unwrap();

    assert_eq!(ordered[0].id, first.id);
    let _ = fs::remove_dir_all(root);
}
