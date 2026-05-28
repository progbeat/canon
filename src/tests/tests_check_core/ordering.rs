use crate::check::run_check_with_runner;
use crate::check_order_state::{latest_recorded_non_pass_timestamp, write_latest_non_pass_record};
use crate::check_selection::order_expectations_by_latest_non_pass;
use crate::fs_util::ensure_dir;
use crate::hash::full_scope;
use crate::history::HistoryCache;
use crate::history_append::append_history_record;
use crate::logging::render_runtime_log_event;
use crate::logging::DiagnosticLogWriter;
use crate::tests::{
    answer, append_legacy_history_record, check_config_yaml, check_options, enable_diagnostic_logs,
    expectation_record, git_project, parse_check_config, FakeRunner,
};
use crate::visible_tree_oid::staged_visible_tree_oid;
use crate::{ERROR_UNPARSABLE, RESULT_FAIL};
use serde_json::json;
use std::fs;
use std::io::{self, Write};

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
        "yes",
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap(),
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
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap(),
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
fn check_runner_skips_all_cached_passes_before_evaluation() {
    let root = git_project("check-order-before-cache-skip");
    enable_diagnostic_logs(&root);
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &[], true, false);
    let first = options.selected[0].clone();
    let second = options.selected[1].clone();
    let current_visible_tree_oid =
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &first,
        &expectation_record(
            &config.agent,
            &first,
            "pass",
            "yes",
            current_visible_tree_oid.clone(),
        ),
    )
    .unwrap();
    append_history_record(
        &root,
        &second,
        &expectation_record(
            &config.agent,
            &second,
            "pass",
            "no",
            current_visible_tree_oid.clone(),
        ),
    )
    .unwrap();
    let mut non_pass = expectation_record(
        &config.agent,
        &second,
        "fail",
        "yes",
        current_visible_tree_oid,
    );
    non_pass.timestamp = "2026-01-01T00:00:00Z".to_string();
    write_latest_non_pass_record(&root, &second, &non_pass).unwrap();
    let mut runner = FakeRunner::new(&[]);
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

    assert_eq!(report.selected, 0);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.silent, 0);
    assert_eq!(report.cached.len(), 2);
    assert_eq!(runner.starts, 0);
    let log = fs::read_to_string(diagnostic_log.path()).unwrap();
    assert!(log.contains(&format!(r#""id":"{}""#, first.id)));
    assert!(log.contains(&format!(r#""id":"{}""#, second.id)));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_orders_cached_failures_by_latest_non_pass_state() {
    let root = git_project("check-order-cached-failures");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &[], true, false);
    let first = options.selected[0].clone();
    let second = options.selected[1].clone();
    let current_visible_tree_oid =
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap();

    let mut first_cached = expectation_record(
        &config.agent,
        &first,
        "fail",
        "no",
        current_visible_tree_oid.clone(),
    );
    first_cached.timestamp = "2026-01-03T00:00:00Z".to_string();
    append_history_record(&root, &first, &first_cached).unwrap();

    let mut second_cached = expectation_record(
        &config.agent,
        &second,
        "fail",
        "yes",
        current_visible_tree_oid,
    );
    second_cached.timestamp = "2026-01-01T00:00:00Z".to_string();
    append_history_record(&root, &second, &second_cached).unwrap();

    let mut second_latest = second_cached.clone();
    second_latest.timestamp = "2026-01-04T00:00:00Z".to_string();
    write_latest_non_pass_record(&root, &second, &second_latest).unwrap();
    let mut runner = FakeRunner::new(&[]);

    let report =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(report.records.len(), 2);
    assert_eq!(report.records[0].id, second.id);
    assert_eq!(report.records[1].id, first.id);
    assert_eq!(runner.starts, 0);
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
        ERROR_UNPARSABLE,
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap(),
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
fn selected_expectations_use_legacy_history_errors_for_order() {
    let root = git_project("check-order-legacy-history-errors");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let first = options.selected[0].clone();
    let second = options.selected[1].clone();
    let mut record = expectation_record(
        &config.agent,
        &second,
        "fail",
        ERROR_UNPARSABLE,
        staged_visible_tree_oid(&root, &config.agent, &full_scope()).unwrap(),
    );
    record.timestamp = "2026-01-01T00:00:00Z".to_string();
    record.error = Some(ERROR_UNPARSABLE.to_string());
    append_legacy_history_record(&root, &second, &record);
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
fn check_runner_records_technical_errors_for_order() {
    let root = git_project("check-order-technical-error");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let first = options.selected[0].clone();
    let mut runner = FakeRunner::new(&[&answer("yes", "passing answer", &["."])]);
    let mut output = FailingWriter;

    let err = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        None,
        Some(&mut output),
    )
    .unwrap_err();

    assert!(err.error.contains("output failed"));
    assert!(latest_recorded_non_pass_timestamp(&root, &first)
        .unwrap()
        .is_some());
    let _ = fs::remove_dir_all(root);
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("output failed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
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
            ("observed", json!(ERROR_UNPARSABLE)),
            ("evidence", json!("unparsable")),
            ("scope", json!(full_scope())),
            ("prompt", json!(second.q.clone())),
            ("expected", json!(second.a.clone())),
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
