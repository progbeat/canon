use super::*;

#[test] // xpec: Eg,90
fn non_live_report_result_output_matches_documented_record_shape() {
    let mut bytes = Vec::new();
    let mut result_output = Some(&mut bytes as &mut dyn Write);

    write_result_output_without_started_report(&mut result_output, &passing_record()).unwrap();

    let rendered = String::from_utf8(bytes).unwrap();
    assert_result_entry(&rendered, "OK");
}

#[test] // xpec: 2gZ,90
fn elapsed_caller_result_output_contains_each_due_default_marker() {
    let mut bytes = Vec::new();
    let mut result_output = Some(&mut bytes as &mut dyn Write);

    write_caller_result_output_with_elapsed_timeline(
        &mut result_output,
        &passing_record(),
        Duration::from_secs(120),
    )
    .unwrap();

    assert_eq!(String::from_utf8(bytes).unwrap(), "j... OK\n");
}

#[test] // xpec: kK
fn summary_output_matches_documented_line() {
    let report = CheckRunReport {
        records: vec![passing_record()],
        cached_passes: Vec::new(),
        pending: 2,
    };
    let mut summary_bytes = Vec::new();

    write_summary_line(&mut summary_bytes, &report, Duration::from_millis(1250)).unwrap();

    let summary = String::from_utf8(summary_bytes).unwrap();
    assert!(summary.contains(" 1 passed, 2 pending in 1.25s "));
    assert!(summary.starts_with('='));
    assert!(summary.ends_with("=\n"));
}

#[test] // xpec: Eg,90
fn failed_result_output_matches_documented_detail_lines() {
    let mut bytes = Vec::new();
    let mut result_output = Some(&mut bytes as &mut dyn Write);
    let mut record = failed_record();
    record.diff_from = Some(":against-tree".to_string());
    record.diff_from_tree_oid = Some("1234567890abcdef1234567890abcdef12345678".to_string());
    record.diff_from_tree_oid_abbrev = Some("1234567".to_string());

    write_result_output_without_started_report(&mut result_output, &record).unwrap();

    let rendered = String::from_utf8(bytes).unwrap();
    assert_result_entry(&rendered, "FAIL");
    assert!(rendered.contains("Does it pass?\n"));
    assert!(rendered.contains("diff-from: 1234567 (:against-tree)\n"));
    assert!(rendered.contains("expected: yes\n"));
    assert!(rendered.contains("observed: no\n"));
    assert!(rendered.contains("evidence: test evidence\n"));
}

#[test] // xpec: Eg,90
fn error_result_output_matches_documented_detail_lines() {
    let mut bytes = Vec::new();
    let mut result_output = Some(&mut bytes as &mut dyn Write);
    let mut record = review_record_with_id("11111111111111111111", "j");
    record.q_scope_suggestion = Some(vec!["src/check".to_string()]);
    record.diff_from = Some(":checkpoint".to_string());
    record.diff_from_tree_oid = Some("abcdef1234567890abcdef1234567890abcdef12".to_string());
    record.diff_from_tree_oid_abbrev = Some("abcdef1".to_string());

    write_result_output_without_started_report(&mut result_output, &record).unwrap();

    let rendered = String::from_utf8(bytes).unwrap();
    assert_result_entry(&rendered, "FAIL");
    assert!(rendered.contains("error: InvalidQuestion\n"));
    assert!(rendered.contains("evidence: test evidence\n"));
    assert!(!rendered.contains("expected:"));
    assert!(!rendered.contains("observed:"));
}

#[test] // xpec: Eg,90
fn live_report_result_output_matches_documented_record_shape() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let output = SharedCheckOutput::new(Box::new(CapturedOutput {
        bytes: bytes.clone(),
    }));

    let report = publish_expectation_report(output, "j");
    let _ = report.append_result(&passing_record());

    let completed = captured_string(&bytes);
    assert_result_entry(&completed, "OK");
}

#[test] // xpec: sy,1h
fn live_report_flushes_short_id_before_first_progress_marker() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let output = SharedCheckOutput::new(Box::new(CapturedOutput {
        bytes: bytes.clone(),
    }));

    let report = publish_expectation_report(output, "j");

    assert_eq!(captured_string(&bytes), "j");
    let _ = report.append_result(&passing_record());
}
