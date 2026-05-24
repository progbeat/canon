use super::*;

#[test]
fn check_runner_reuses_exact_cached_failure_after_cooldown_miss() {
    let root = git_project("check-cooldown-fail-exact-cache");
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
    let options = check_options(&config, &["1"], false, false);
    let expectation = options.selected[0].clone();
    let current_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:20Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Fail,
            prompt: Some(expectation.q.clone()),
            expected: Some(expectation.a.clone()),
            observed: "no".to_string(),
            evidence: "exact cached failure".to_string(),
            scope: full_scope(),
            scope_hash: current_hash,
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(records.records.len(), 1);
    assert_eq!(records.evaluated, 0);
    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].evidence, "exact cached failure");
    assert_eq!(runner.starts, 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_skips_cached_pass_without_result_output() {
    let root = git_project("check-cache-pass-output");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], true, false);
    let expectation = options.selected[0].clone();
    let scope_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:20Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: Some(expectation.q.clone()),
            expected: Some(expectation.a.clone()),
            observed: "yes".to_string(),
            evidence: "cached pass".to_string(),
            scope: full_scope(),
            scope_hash,
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[]);
    let mut output = FlushCountingWriter::new();

    let report = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        None,
        Some(&mut output),
    )
    .unwrap();

    assert_eq!(report.selected, 1);
    assert_eq!(report.skipped, 1);
    assert_eq!(report.silent, 1);
    assert_eq!(report_output_skipped_count(&report), 1);
    assert_eq!(runner.starts, 0);
    assert_eq!(output.flushes, 0);
    assert!(output.bytes.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_all_reports_cached_pass_without_silent_skip() {
    let root = git_project("check-all-cache-pass-output");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, false);
    let expectation = options.selected[0].clone();
    let scope_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:20Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: Some(expectation.q.clone()),
            expected: Some(expectation.a.clone()),
            observed: "yes".to_string(),
            evidence: "cached pass".to_string(),
            scope: full_scope(),
            scope_hash,
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[]);
    let mut output = FlushCountingWriter::new();

    let report = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        None,
        Some(&mut output),
    )
    .unwrap();

    assert_eq!(report.records.len(), 1);
    assert!(report.records[0].passed());
    assert_eq!(report.selected, 1);
    assert_eq!(report.skipped, 1);
    assert_eq!(report.silent, 0);
    assert_eq!(report.evaluated, 0);
    assert_eq!(runner.starts, 0);
    let lines = String::from_utf8(output.bytes).unwrap();
    assert!(lines.contains(&format!("{}. OK", expectation.display_id)));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_deselects_cached_pass_when_no_selectors_are_given() {
    let root = git_project("check-cache-pass-default-selection");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &[], true, false);
    let expectation = options.selected[0].clone();
    let scope_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:20Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: Some(expectation.q.clone()),
            expected: Some(expectation.a.clone()),
            observed: "yes".to_string(),
            evidence: "cached pass".to_string(),
            scope: full_scope(),
            scope_hash,
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[&answer("no", "README.md says no", &["."])]);
    let mut output = FlushCountingWriter::new();

    let report = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        None,
        Some(&mut output),
    )
    .unwrap();

    assert_eq!(report.records.len(), 1);
    assert_eq!(report.records[0].number, 2);
    assert_eq!(report.selected, 2);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.silent, 1);
    assert_eq!(runner.starts, 1);
    let lines = String::from_utf8(output.bytes).unwrap();
    assert!(lines.contains(&format!("{}. OK", report.records[0].display_id)));
    assert!(!lines.contains(&format!("{}. OK", expectation.display_id)));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_deselects_fresh_cooldown_pass_before_cache_reuse() {
    let root = git_project("check-cooldown-deselect");
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
    let options = check_options(&config, &[], true, false);
    let expectation = options.selected[0].clone();
    let old_hash = "old-scope".to_string();
    let mut record = expectation_record(&config.agent, &expectation, "pass", "yes", old_hash);
    record.timestamp = format_record_timestamp(unix_timestamp().unwrap());
    append_history_record(&root, &expectation, &record).unwrap();
    let mut runner = FakeRunner::new(&[]);
    let mut output = FlushCountingWriter::new();

    let report = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        None,
        Some(&mut output),
    )
    .unwrap();

    assert!(report.records.is_empty());
    assert_eq!(report.selected, 1);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.silent, 1);
    assert_eq!(report_output_skipped_count(&report), 0);
    assert_eq!(runner.starts, 0);
    assert!(output.bytes.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_all_runs_fresh_cooldown_pass() {
    let root = git_project("check-all-cooldown-runs");
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
    let options = check_options(&config, &[], false, false);
    let expectation = options.selected[0].clone();
    let mut record = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        "old-scope".to_string(),
    );
    record.timestamp = format_record_timestamp(unix_timestamp().unwrap());
    append_history_record(&root, &expectation, &record).unwrap();
    let mut runner = FakeRunner::new(&[&answer("yes", "fresh answer", &["."])]);

    let report =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(report.records.len(), 1);
    assert_eq!(report.evaluated, 1);
    assert_eq!(report.selected, 1);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.silent, 0);
    assert_eq!(runner.starts, 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_ignore_cache_still_deselects_fresh_cooldown_pass() {
    let root = git_project("check-cooldown-ignore-cache");
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
    let options = check_options(&config, &[], true, true);
    let expectation = options.selected[0].clone();
    let old_hash = "old-scope".to_string();
    let mut record = expectation_record(&config.agent, &expectation, "pass", "yes", old_hash);
    record.timestamp = format_record_timestamp(unix_timestamp().unwrap());
    append_history_record(&root, &expectation, &record).unwrap();
    let mut runner = FakeRunner::new(&[]);

    let report =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(report.records.len(), 0);
    assert_eq!(report.selected, 1);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.silent, 1);
    assert_eq!(report_output_skipped_count(&report), 0);
    assert_eq!(runner.starts, 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_ignore_cooldown_runs_fresh_cooldown_pass() {
    let root = git_project("check-cooldown-ignore-cooldown");
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
    let mut options = check_options(&config, &[], true, true);
    options.ignore_cooldown = true;
    let expectation = options.selected[0].clone();
    let old_hash = "old-scope".to_string();
    let mut record = expectation_record(&config.agent, &expectation, "pass", "yes", old_hash);
    record.timestamp = format_record_timestamp(unix_timestamp().unwrap());
    append_history_record(&root, &expectation, &record).unwrap();
    let mut runner = FakeRunner::new(&[&answer("yes", "fresh answer", &["."])]);

    let report =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(report.records.len(), 1);
    assert_eq!(report.evaluated, 1);
    assert_eq!(report.selected, 1);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.silent, 0);
    assert_eq!(runner.starts, 1);
    let _ = fs::remove_dir_all(root);
}
