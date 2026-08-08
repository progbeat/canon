use super::{
    finish_query_output, publish_expectation_report, start_query_report_output,
    summary_outcome_counts, write_caller_result_output_with_elapsed_timeline,
    write_result_output_without_started_report, write_summary_line, SharedCheckOutput,
};
use crate::check::core::{
    CheckRecord, CheckResult, CheckRunReport, EvaluationAnswer, ParsedAnswer, QueryResult,
    ERROR_INVALID_QUESTION,
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
struct CapturedOutput {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl Write for CapturedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

mod messages;
mod records;

fn assert_result_entry(rendered: &str, status: &str) {
    let first_line = rendered.lines().next().expect("result entry line");
    let (id_and_dots, observed_status) = first_line
        .split_once(' ')
        .expect("result entry separates id/dots from status");
    // xpec: 2gZ,Eg,90
    assert_eq!(id_and_dots.trim_end_matches('.'), "j");
    // xpec: 2gZ,Eg,90
    assert!(id_and_dots.ends_with('.'));
    // xpec: 2gZ,Eg,90
    assert_eq!(observed_status, status);
}

fn captured_string(bytes: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8(bytes.lock().unwrap().clone()).unwrap()
}

fn passing_record() -> CheckRecord {
    record_with_result(CheckResult::Pass, "yes")
}

fn failed_record() -> CheckRecord {
    record_with_result(CheckResult::Fail, "no")
}

fn failed_record_with_id(id: &str, display_id: &str) -> CheckRecord {
    record_with_identity(CheckResult::Fail, "no", None, id, display_id)
}

fn passing_record_with_id(id: &str, display_id: &str) -> CheckRecord {
    record_with_identity(CheckResult::Pass, "yes", None, id, display_id)
}

fn review_record_with_id(id: &str, display_id: &str) -> CheckRecord {
    record_with_identity(
        CheckResult::Fail,
        "",
        Some(ERROR_INVALID_QUESTION),
        id,
        display_id,
    )
}

fn record_with_result(result: CheckResult, observed: &str) -> CheckRecord {
    record_with_identity(result, observed, None, "11111111111111111111", "j")
}

fn record_with_identity(
    result: CheckResult,
    observed: &str,
    error: Option<&str>,
    id: &str,
    display_id: &str,
) -> CheckRecord {
    CheckRecord {
        timestamp: "1970-01-01T00:00:00Z".to_string(),
        result,
        to: crate::config_types::ExpectationTo::Agent,
        question: Some("Does it pass?".to_string()),
        expected_answer: Some("yes".to_string()),
        observed: observed.to_string(),
        error: error.map(str::to_string),
        evidence: Some("test evidence".to_string()),
        scope: vec![".".to_string()],
        q_scope_suggestion: None,
        visible_tree_oid: Some("visible".to_string()),
        diff_from: None,
        diff_from_tree_oid: None,
        diff_from_tree_oid_abbrev: None,
        id: id.to_string(),
        display_id: display_id.to_string(),
    }
}
