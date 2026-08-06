use super::progress::publish_expectation_report;
use super::SharedCheckOutput;
use crate::check::core::{CheckRecord, CheckResult, INTERNAL_ERROR_UNPARSABLE};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

#[test] // xpec: Eg
fn interactive_caller_result_uses_the_required_renderer_safe_line_ending() {
    let mut record = timeout_error_record();
    record.result = CheckResult::Pass;
    record.to = crate::config_types::ExpectationTo::Caller;
    record.observed = "yes".to_string();
    record.error = None;
    record.evidence = None;
    let mut bytes = Vec::new();
    let mut output: Option<&mut dyn Write> = Some(&mut bytes);

    super::write_caller_result_output_with_interactivity(
        &mut output,
        &record,
        std::time::Duration::ZERO,
        true,
    )
    .unwrap();

    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        "\u{1b}[u\u{1b}[0J\r\u{1b}[0mj. OK\u{1b}[8m\n\u{1b}[0m"
    );
}

#[test] // xpec: 90
fn check_error_output_prints_error_without_inline_escaping() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let output = SharedCheckOutput::new(Box::new(CapturedOutput {
        bytes: bytes.clone(),
    }));
    let report = publish_expectation_report(output, "j");
    let mut record = timeout_error_record();
    record.error = Some("first\nsecond".to_string());

    let _ = report.append_result(&record);

    assert!(String::from_utf8(bytes.lock().unwrap().clone())
        .unwrap()
        .contains("error: first\nsecond\n"));
}

#[test] // xpec: 90
fn check_diff_from_output_is_not_inline_escaped() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let output = SharedCheckOutput::new(Box::new(CapturedOutput {
        bytes: bytes.clone(),
    }));
    let report = publish_expectation_report(output, "j");
    let mut record = timeout_error_record();
    record.error = None;
    record.observed = "no".to_string();
    record.diff_from = Some("head\nbase".to_string());
    record.diff_from_tree_oid_abbrev = Some("abc123".to_string());

    let _ = report.append_result(&record);

    assert!(String::from_utf8(bytes.lock().unwrap().clone())
        .unwrap()
        .contains("diff-from: abc123 (head\nbase)\n"));
}

#[test] // xpec: sy
fn printed_short_id_is_a_public_report_when_stdout_result_append_fails() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let output = SharedCheckOutput::new(Box::new(FailAfterFirstFlushOutput {
        bytes: bytes.clone(),
        first_flush_completed: false,
    }));
    let report = publish_expectation_report(output, "j");

    let _ = report.append_result(&timeout_error_record());

    assert_eq!(*bytes.lock().unwrap(), b"j");
}

#[test] // xpec: sy
fn fully_written_short_id_is_a_public_report_when_flush_and_result_append_fail() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let output = SharedCheckOutput::new(Box::new(FailAfterWritingShortIdOutput {
        bytes: bytes.clone(),
        flush_was_attempted: false,
    }));
    let report = publish_expectation_report(output, "jK9");

    let outcome = report.append_result(&timeout_error_record());

    assert!(outcome.anything_was_reported());
    assert_eq!(*bytes.lock().unwrap(), b"jK9");
}

#[test] // xpec: 2gZ,sy
fn failed_initial_short_id_fallback_keeps_elapsed_and_final_markers() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let output = SharedCheckOutput::new(Box::new(FailFirstWriteOutput {
        bytes: bytes.clone(),
        first_write: true,
    }));
    let report = publish_expectation_report(output, "j");
    report
        .record_elapsed_marker_for_test(
            crate::evaluator::EvaluatorProgressMarker::NoHigherPriorityEvent,
        )
        .unwrap();

    let outcome = report.append_result(&timeout_error_record());

    assert!(outcome.anything_was_reported());
    assert_eq!(
        *bytes.lock().unwrap(),
        b"j.. FAIL\nerror: unparsable\nevidence: technical timeout\n"
    );
}

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

struct FailAfterFirstFlushOutput {
    bytes: Arc<Mutex<Vec<u8>>>,
    first_flush_completed: bool,
}

struct FailAfterWritingShortIdOutput {
    bytes: Arc<Mutex<Vec<u8>>>,
    flush_was_attempted: bool,
}

struct FailFirstWriteOutput {
    bytes: Arc<Mutex<Vec<u8>>>,
    first_write: bool,
}

impl Write for FailFirstWriteOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.first_write {
            self.first_write = false;
            return Err(io::Error::other("first write failed"));
        }
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Write for FailAfterWritingShortIdOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.flush_was_attempted {
            return Err(io::Error::other("stdout unavailable after short ID"));
        }
        self.bytes.lock().unwrap().push(bytes[0]);
        Ok(1)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_was_attempted = true;
        Err(io::Error::other("stdout cannot flush"))
    }
}

impl Write for FailAfterFirstFlushOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.first_flush_completed {
            return Err(io::Error::other("stdout unavailable after short ID"));
        }
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.first_flush_completed = true;
        Ok(())
    }
}

fn timeout_error_record() -> CheckRecord {
    CheckRecord {
        timestamp: "1970-01-01T00:00:00Z".to_string(),
        result: CheckResult::Fail,
        to: crate::config_types::ExpectationTo::Agent,
        question: Some("Does it pass?".to_string()),
        expected_answer: Some("yes".to_string()),
        observed: INTERNAL_ERROR_UNPARSABLE.to_string(),
        error: Some(INTERNAL_ERROR_UNPARSABLE.to_string()),
        evidence: Some("technical timeout".to_string()),
        scope: vec![".".to_string()],
        q_scope_suggestion: None,
        visible_tree_oid: Some("visible".to_string()),
        diff_from: None,
        diff_from_tree_oid: None,
        diff_from_tree_oid_abbrev: None,
        id: "11111111111111111111".to_string(),
        display_id: "j".to_string(),
    }
}
