use crate::check::core::types::{is_line_break_char, CheckRecord, CheckRunReport, ParsedAnswer};
use crate::json_util::compact_json_string_array;
use crate::logs::push_json_control_escape;
use crate::token_usage_types::TokenUsage;
use std::collections::BTreeSet;
use std::io::{self, Write};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const ALL_CHECKS_PASSED_MESSAGE: &str = "✓ All checks passed. Commit is allowed.";
const FIX_ISSUES_MESSAGE: &str = "▷ Fix the issues and run `canon check` again!";
const THEN_FIX_REMAINING_MESSAGE: &str =
    "▷ Then fix the remaining issues and run `canon check` again!";
const PASS_IMPROVEMENT_COMMIT_SUFFIX: &str = "Commit the staged changes NOW!";
const PROGRESS_DOT_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub(crate) struct SharedCheckOutput {
    inner: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl SharedCheckOutput {
    pub(crate) fn stdout() -> SharedCheckOutput {
        SharedCheckOutput::new(Box::new(io::stdout()))
    }

    pub(crate) fn new(writer: Box<dyn Write + Send>) -> SharedCheckOutput {
        SharedCheckOutput {
            inner: Arc::new(Mutex::new(writer)),
        }
    }
}

impl Write for SharedCheckOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut writer = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("check output lock poisoned"))?;
        writer.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut writer = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("check output lock poisoned"))?;
        writer.flush()
    }
}

pub(crate) struct CheckProgressOutput {
    output: SharedCheckOutput,
    stop: Sender<()>,
    worker: Option<JoinHandle<Result<(), String>>>,
}

pub(crate) fn start_check_progress_output(
    output: SharedCheckOutput,
    display_id: &str,
) -> Result<CheckProgressOutput, String> {
    let mut immediate_output = output.clone();
    write_stdout_record(
        &mut immediate_output,
        format!("{}.", display_id).as_bytes(),
        "check progress prefix",
    )?;

    let (stop, stop_requested) = mpsc::channel();
    let mut progress_output = output.clone();
    let worker = thread::spawn(move || loop {
        match stop_requested.recv_timeout(PROGRESS_DOT_INTERVAL) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {
                write_stdout_record(&mut progress_output, b".", "check progress dot")?;
            }
        }
    });

    Ok(CheckProgressOutput {
        output,
        stop,
        worker: Some(worker),
    })
}

impl CheckProgressOutput {
    pub(crate) fn finish_with_record(mut self, record: &CheckRecord) -> Result<(), String> {
        self.stop_progress_worker()?;
        let completion = render_check_output_record_completion(record);
        let mut output = self.output.clone();
        write_stdout_record(&mut output, completion.as_bytes(), "check result")
    }

    pub(crate) fn cancel(mut self) -> Result<(), String> {
        self.stop_progress_worker()
    }

    fn stop_progress_worker(&mut self) -> Result<(), String> {
        let _ = self.stop.send(());
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker
            .join()
            .map_err(|_| "check progress thread panicked".to_string())?
    }
}

pub(crate) fn write_and_flush_result_output(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
    elapsed: Duration,
) -> Result<(), String> {
    if let Some(writer) = result_output.as_mut() {
        let line = render_check_output_record(record, elapsed);
        write_stdout_record(*writer, line.as_bytes(), "check result")?;
    }
    Ok(())
}

pub(crate) fn write_summary_line(
    result_output: &mut dyn Write,
    report: &CheckRunReport,
    elapsed: Duration,
) -> Result<(), String> {
    let line = render_check_summary(report, elapsed);
    write_stdout_record(result_output, line.as_bytes(), "check summary")
}

pub(crate) fn write_query_output(
    result_output: &mut dyn Write,
    answer: &ParsedAnswer,
) -> Result<(), String> {
    // Query output is intentionally separate from the selected-expectation
    // check output contract because query mode has no expectation selector,
    // expected answer, reusable history write, or final check summary.
    let output = render_query_output(answer);
    write_stdout_record(result_output, output.as_bytes(), "query result")
}

pub(crate) fn write_stdout_line_record(
    writer: &mut dyn Write,
    line: &str,
    description: &str,
) -> Result<(), String> {
    let mut output = String::with_capacity(line.len() + 1);
    output.push_str(line);
    output.push('\n');
    write_stdout_record(writer, output.as_bytes(), description)
}

fn write_stdout_record(
    writer: &mut dyn Write,
    bytes: &[u8],
    description: &str,
) -> Result<(), String> {
    writer
        .write_all(bytes)
        .map_err(|err| format!("failed to write {} to stdout: {}", description, err))?;
    writer
        .flush()
        .map_err(|err| format!("failed to flush {} to stdout: {}", description, err))
}

pub(crate) fn render_query_output(answer: &ParsedAnswer) -> String {
    let mut output = String::new();
    if let Some(error) = answer.error.as_deref() {
        output.push_str("Error: ");
        output.push_str(&escape_check_output_text(error));
    } else {
        output.push_str("Observed: ");
        output.push_str(&escape_check_output_text(&answer.answer));
    }
    output.push('\n');
    output.push_str("Evidence: ");
    output.push_str(&escape_check_output_text(&answer.evidence));
    output.push('\n');
    if let Some(suggestion) = answer.q_scope_suggestion.as_deref() {
        output.push_str("Suggested q-scope: ");
        output.push_str(&compact_json_string_array(suggestion));
        output.push('\n');
    }
    output
}

pub(crate) fn render_check_output_record(record: &CheckRecord, elapsed: Duration) -> String {
    let dots = result_elapsed_dots(elapsed);
    let mut output = format!("{}{}", record.display_id, dots);
    output.push_str(&render_check_output_record_completion(record));
    output
}

fn render_check_output_record_completion(record: &CheckRecord) -> String {
    if record.passed() {
        return " OK\n".to_string();
    }
    let is_error = record_requires_human_review(record);
    let status = if is_error { "ERROR" } else { "FAILED" };
    let mut output = String::new();
    output.push_str(&format!(" {}\n", status));
    // This is the spec's `<escaped question>` line, not an extra line beyond
    // the six-line failed and five-line error layouts.
    output.push_str(&escape_check_output_text(record.prompt_text()));
    output.push('\n');
    if is_error {
        output.push_str("Error: ");
        let error = record
            .review_error_text()
            .expect("error records must expose an error value");
        output.push_str(&escape_check_output_text(error));
        output.push('\n');
    } else {
        output.push_str("Expected: ");
        output.push_str(&escape_check_output_text(
            record.expected_text().unwrap_or(""),
        ));
        output.push('\n');
        output.push_str("Observed: ");
        output.push_str(&escape_check_output_text(&record.observed));
        output.push('\n');
    }
    output.push_str("Evidence: ");
    output.push_str(&escape_check_output_text(&record.evidence));
    output.push('\n');
    if !is_error {
        if let Some(suggestion) = record.suggested_q_scope.as_deref() {
            output.push_str("Suggested q-scope: ");
            output.push_str(&compact_json_string_array(suggestion));
            output.push('\n');
        }
    }
    output
}

fn result_elapsed_dots(elapsed: Duration) -> String {
    ".".repeat(result_elapsed_dot_count(elapsed))
}

fn result_elapsed_dot_count(elapsed: Duration) -> usize {
    const NANOS_PER_MINUTE: u128 = 60 * 1_000_000_000;
    let elapsed_nanos = elapsed.as_nanos();
    let mut dots = elapsed_nanos / NANOS_PER_MINUTE;
    if !elapsed_nanos.is_multiple_of(NANOS_PER_MINUTE) {
        dots += 1;
    }
    usize::try_from(dots.max(1)).unwrap_or(usize::MAX)
}

pub(crate) fn render_token_usage_summary(usage: TokenUsage) -> String {
    format!(
        "Token usage: total={} input={} (+ {} cached) output={} (reasoning {})",
        usage.total_tokens,
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.output_tokens,
        usage.reasoning_output_tokens
    )
}

pub(crate) fn render_check_summary(report: &CheckRunReport, elapsed: Duration) -> String {
    // Summary order is fixed to match the spec and pytest-style labels:
    // failed, error/errors, passed, skipped.
    // Cached passes are part of the pass category even though they have no
    // per-expectation stdout line.
    let SummaryOutcomeCounts {
        passed,
        failed,
        errors,
    } = summary_outcome_counts(report);
    let mut outcomes = Vec::new();
    if failed > 0 {
        outcomes.push(format!("{} failed", failed));
    }
    if errors > 0 {
        outcomes.push(format!(
            "{} {}",
            errors,
            if errors == 1 { "error" } else { "errors" }
        ));
    }
    if passed > 0 {
        outcomes.push(format!("{} passed", passed));
    }
    // `skipped` is the count of expectations outside the summary categories.
    // Cached passes have no per-expectation stdout line, but they are still
    // pass results and therefore are not skipped in the summary.
    let skipped = report.skipped;
    if skipped > 0 {
        outcomes.push(format!("{} skipped", skipped));
    }
    if outcomes.is_empty() {
        outcomes.push("0 passed".to_string());
    }
    let inner = format!(" {} in {:.2}s ", outcomes.join(", "), elapsed.as_secs_f64());
    format!("{}\n", pad_summary_line(&inner))
}

pub(crate) fn render_check_agent_messages(
    num_failed: usize,
    num_errors: usize,
    num_fixes: usize,
    num_regressions: usize,
) -> Vec<String> {
    let num_issues = num_failed + num_errors;
    if num_regressions > 0 || (num_issues > 0 && num_fixes == 0) {
        return vec![FIX_ISSUES_MESSAGE.to_string()];
    }
    if num_issues == 0 && num_fixes == 0 {
        return vec![ALL_CHECKS_PASSED_MESSAGE.to_string()];
    }

    let mut messages = vec![pass_improvement_notice(num_fixes).expect("positive fix count")];
    if num_issues > 0 {
        messages.push(THEN_FIX_REMAINING_MESSAGE.to_string());
    }
    messages
}

fn pass_improvement_notice(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some(format!(
            "▷ +1 pass compared to HEAD. {}",
            PASS_IMPROVEMENT_COMMIT_SUFFIX
        )),
        count => Some(format!(
            "▷ +{} passes compared to HEAD. {}",
            count, PASS_IMPROVEMENT_COMMIT_SUFFIX
        )),
    }
}

pub(crate) struct SummaryOutcomeCounts {
    pub(crate) passed: usize,
    pub(crate) failed: usize,
    pub(crate) errors: usize,
}

pub(crate) fn summary_outcome_counts(report: &CheckRunReport) -> SummaryOutcomeCounts {
    let mut counts = SummaryOutcomeCounts {
        passed: 0,
        failed: 0,
        errors: 0,
    };
    let mut seen = BTreeSet::new();
    for record in &report.records {
        if seen.insert(record.id.clone()) {
            add_summary_record(&mut counts, record);
        }
    }
    for cached in &report.cached {
        let id = if cached.record.id.is_empty() {
            &cached.expectation.id
        } else {
            &cached.record.id
        };
        if seen.insert(id.clone()) {
            add_summary_record(&mut counts, &cached.record);
        }
    }
    counts
}

fn add_summary_record(counts: &mut SummaryOutcomeCounts, record: &CheckRecord) {
    if record.passed() {
        counts.passed += 1;
    } else if record_requires_human_review(record) {
        counts.errors += 1;
    } else {
        counts.failed += 1;
    }
}

pub(crate) fn pad_summary_line(inner: &str) -> String {
    const WIDTH: usize = 80;
    // Reserve at least one `=` on each side even when the outcome text is
    // wider than the usual summary width.
    let width = WIDTH.max(inner.len() + 2);
    let padding = width - inner.len();
    let left = padding / 2;
    let right = padding - left;
    format!("{}{}{}", "=".repeat(left), inner, "=".repeat(right))
}

pub(crate) fn record_requires_human_review(record: &CheckRecord) -> bool {
    record.review_error_text().is_some()
}

pub(crate) fn escape_check_output_text(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        match ch {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if is_line_break_char(ch) || ch.is_control() => {
                push_check_output_unicode_escape(&mut output, ch);
            }
            ch => output.push(ch),
        }
    }
    output
}

fn push_check_output_unicode_escape(output: &mut String, ch: char) {
    if (ch as u32) <= 0xff {
        push_json_control_escape(output, ch as u8);
    } else {
        output.push_str(&format!("\\u{:04x}", ch as u32));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        render_check_agent_messages, start_check_progress_output, write_and_flush_result_output,
        SharedCheckOutput,
    };
    use crate::check::core::types::{CheckRecord, CheckResult};
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

    #[test]
    fn check_result_output_rounds_progress_dots_to_elapsed_minutes() {
        let mut bytes = Vec::new();
        let mut result_output = Some(&mut bytes as &mut dyn Write);

        write_and_flush_result_output(
            &mut result_output,
            &passing_record(),
            Duration::from_secs(60) + Duration::from_nanos(1),
        )
        .unwrap();

        assert_eq!(String::from_utf8(bytes).unwrap(), "j.. OK\n");
    }

    #[test]
    fn progress_output_writes_prefix_before_completion() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = SharedCheckOutput::new(Box::new(CapturedOutput {
            bytes: bytes.clone(),
        }));

        let progress = start_check_progress_output(output, "j").unwrap();
        assert_eq!(captured_string(&bytes), "j.");

        progress.finish_with_record(&passing_record()).unwrap();
        assert_eq!(captured_string(&bytes), "j. OK\n");
    }

    #[test]
    fn check_agent_messages_follow_spec_branch_order() {
        assert_eq!(
            render_check_agent_messages(1, 0, 0, 0),
            vec!["▷ Fix the issues and run `canon check` again!"]
        );
        assert_eq!(
            render_check_agent_messages(0, 0, 0, 0),
            vec!["✓ All checks passed. Commit is allowed."]
        );
        assert_eq!(
            render_check_agent_messages(0, 0, 1, 0),
            vec!["▷ +1 pass compared to HEAD. Commit the staged changes NOW!"]
        );
        assert_eq!(
            render_check_agent_messages(1, 0, 2, 0),
            vec![
                "▷ +2 passes compared to HEAD. Commit the staged changes NOW!",
                "▷ Then fix the remaining issues and run `canon check` again!"
            ]
        );
        assert_eq!(
            render_check_agent_messages(0, 0, 1, 1),
            vec!["▷ Fix the issues and run `canon check` again!"]
        );
    }

    fn captured_string(bytes: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(bytes.lock().unwrap().clone()).unwrap()
    }

    fn passing_record() -> CheckRecord {
        CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: 1,
            result: CheckResult::Pass,
            prompt: Some("Does it pass?".to_string()),
            expected: Some("yes".to_string()),
            observed: "yes".to_string(),
            error: None,
            evidence: "test evidence".to_string(),
            scope: vec![".".to_string()],
            suggested_q_scope: None,
            visible_tree_oid: "visible".to_string(),
            id: "11111111111111111111".to_string(),
            display_id: "j".to_string(),
            cache_key: None,
        }
    }
}
