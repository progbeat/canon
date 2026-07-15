use super::escape::escape_check_output_text;
use super::shared::{write_stdout_record, SharedCheckOutput};
use crate::check::core::{CheckRecord, ERROR_SCOPE_TOO_NARROW};
use crate::config_types::ExpectationTo;
use crate::evaluator::{EvaluatorProgress, EvaluatorProgressMarker};
use crate::json_util::compact_json_string_array;
use std::io::Write;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const PROGRESS_TIMELINE_ELAPSED_MARKER_INTERVAL: Duration = Duration::from_secs(60);

// Progress timeline ownership:
// - `start_expectation_report_output` writes and flushes `<short ID>` before
//   evaluator work starts; that prefix is the started public report for the
//   expectation, not a timeline marker.
// - `start_query_report_output` has no prefix. It only performs the required
//   pre-marker stdout flush for `canon ask`.
// - the progress worker writes and flushes elapsed markers every minute while
//   evaluator work is still active.
// - completion writes the final marker before the result suffix or response
//   block. The final marker is emitted even when the final interval is 0
//   seconds.
// - query completion writes the newline after those markers before the query
//   response, so completed `canon ask` stdout starts with a standalone progress
//   timeline line.
// Event-to-marker priority and symbols live in `src/evaluator/progress.rs`.
pub(crate) struct StartedExpectationReportOutput {
    output: SharedCheckOutput,
    stop: Sender<()>,
    active: Arc<AtomicBool>,
    progress: EvaluatorProgress,
    timeline: Arc<Mutex<ElapsedProgressTimelineState>>,
    worker: Option<JoinHandle<Result<(), String>>>,
    prefix_completed: bool,
}

pub(crate) struct FinishedExpectationReportOutput {
    stdout_completion_failed: bool,
}

impl FinishedExpectationReportOutput {
    fn completed_report() -> FinishedExpectationReportOutput {
        FinishedExpectationReportOutput {
            stdout_completion_failed: false,
        }
    }

    fn with_stdout_completion_failed() -> FinishedExpectationReportOutput {
        FinishedExpectationReportOutput {
            stdout_completion_failed: true,
        }
    }

    pub(crate) fn stdout_completion_failed(&self) -> bool {
        self.stdout_completion_failed
    }
}

struct ElapsedProgressTimelineState {
    next_marker_at: Instant,
    progress_timeline_tail: [Option<EvaluatorProgressMarker>; 2],
}

pub(crate) fn start_expectation_report_output(
    output: SharedCheckOutput,
    display_id: &str,
) -> StartedExpectationReportOutput {
    start_progress_report_output(output, display_id, "started expectation report prefix")
}

pub(crate) fn start_query_report_output(
    output: SharedCheckOutput,
) -> StartedExpectationReportOutput {
    start_progress_report_output(output, "", "started query report timeline")
}

fn start_progress_report_output(
    output: SharedCheckOutput,
    prefix: &str,
    description: &str,
) -> StartedExpectationReportOutput {
    let mut immediate_output = output.clone();
    let prefix_completed =
        write_stdout_record(&mut immediate_output, prefix.as_bytes(), description).is_ok();

    let (stop, stop_requested) = mpsc::channel();
    let active = Arc::new(AtomicBool::new(true));
    let worker_active = active.clone();
    let progress = EvaluatorProgress::new();
    let worker_progress = progress.clone();
    let first_elapsed_marker_at = Instant::now() + PROGRESS_TIMELINE_ELAPSED_MARKER_INTERVAL;
    let elapsed_timeline = Arc::new(Mutex::new(ElapsedProgressTimelineState {
        next_marker_at: first_elapsed_marker_at,
        progress_timeline_tail: [None, None],
    }));
    let worker_timeline = elapsed_timeline.clone();
    let mut progress_output = output.clone();
    let worker = thread::spawn(move || loop {
        match stop_requested.recv_timeout(wait_for_next_elapsed_marker(&worker_timeline)) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {
                if !worker_active.load(Ordering::Acquire) {
                    return Ok(());
                }
                let marker = elapsed_progress_marker_due(
                    &worker_progress,
                    &worker_timeline,
                    Instant::now(),
                )?;
                if let Some(marker) = marker {
                    record_progress_marker(&worker_timeline, marker)?;
                    write_stdout_record(
                        &mut progress_output,
                        marker.as_str().as_bytes(),
                        "check live report progress marker",
                    )?;
                }
            }
        }
    });

    StartedExpectationReportOutput {
        output,
        stop,
        active,
        progress,
        timeline: elapsed_timeline,
        worker: Some(worker),
        prefix_completed,
    }
}

impl StartedExpectationReportOutput {
    pub(crate) fn progress(&self) -> EvaluatorProgress {
        self.progress.clone()
    }

    pub(crate) fn finish_with_record(
        mut self,
        record: &CheckRecord,
    ) -> FinishedExpectationReportOutput {
        // Once the report prefix is visible, final result output has priority
        // over delayed progress-worker cleanup errors.
        let _ = self.stop_progress_worker();
        let mut output = self.output.clone();
        let mut completion_problem = false;
        if self.prefix_completed {
            if self.write_completion_markers(&mut output).is_err() {
                completion_problem = true;
            }
            if assert_final_no_progress_turn_timeout_suffix(&self.timeline, record).is_err() {
                completion_problem = true;
            }
        }
        let result_suffix = if self.prefix_completed {
            render_check_output_record_status_and_details(record)
        } else {
            render_check_output_record_with_initial_marker_timeline(record)
        };
        if write_stdout_record(&mut output, result_suffix.as_bytes(), "check result").is_err() {
            return FinishedExpectationReportOutput::with_stdout_completion_failed();
        }
        if completion_problem {
            return FinishedExpectationReportOutput::with_stdout_completion_failed();
        }
        FinishedExpectationReportOutput::completed_report()
    }

    pub(crate) fn finish_with_query_output(mut self, query_output: &str) -> Result<(), String> {
        let _ = self.stop_progress_worker();
        if !self.prefix_completed {
            return Err("failed to flush started query report timeline to stdout".to_string());
        }
        let mut output = self.output.clone();
        self.write_completion_markers(&mut output)?;
        write_stdout_record(&mut output, b"\n", "query progress timeline newline")?;
        write_stdout_record(&mut output, query_output.as_bytes(), "query result")
    }

    fn stop_progress_worker(&mut self) -> Result<(), String> {
        self.active.store(false, Ordering::Release);
        let _ = self.stop.send(());
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker
            .join()
            .map_err(|_| "check live report progress thread panicked".to_string())?
    }

    fn due_elapsed_progress_markers(&self) -> Result<Vec<EvaluatorProgressMarker>, String> {
        completion_progress_markers_due(&self.progress, &self.timeline, Instant::now())
    }

    fn write_completion_markers(&self, output: &mut SharedCheckOutput) -> Result<(), String> {
        for marker in self.due_elapsed_progress_markers()? {
            record_progress_marker(&self.timeline, marker)?;
            write_stdout_record(
                output,
                marker.as_str().as_bytes(),
                "check live report progress marker",
            )?;
        }
        Ok(())
    }
}

fn record_progress_marker(
    timeline: &Arc<Mutex<ElapsedProgressTimelineState>>,
    marker: EvaluatorProgressMarker,
) -> Result<(), String> {
    let mut timeline = timeline
        .lock()
        .map_err(|_| "check live report progress state poisoned".to_string())?;
    let next_tail = [timeline.progress_timeline_tail[1], Some(marker)];
    timeline.progress_timeline_tail = next_tail;
    Ok(())
}

fn assert_final_no_progress_turn_timeout_suffix(
    timeline: &Arc<Mutex<ElapsedProgressTimelineState>>,
    record: &CheckRecord,
) -> Result<(), String> {
    if !record.requires_human_review() {
        return Ok(());
    }
    let timeline = timeline
        .lock()
        .map_err(|_| "check live report progress state poisoned".to_string())?;
    if timeline.progress_timeline_tail[1] == Some(EvaluatorProgressMarker::TurnTimeout) {
        assert_progress_timeline_suffix_is_idle_then_timeout(timeline.progress_timeline_tail)?;
    }
    Ok(())
}

fn assert_progress_timeline_suffix_is_idle_then_timeout(
    progress_timeline_tail: [Option<EvaluatorProgressMarker>; 2],
) -> Result<(), String> {
    let expected = [
        Some(EvaluatorProgressMarker::Idle),
        Some(EvaluatorProgressMarker::TurnTimeout),
    ];
    if progress_timeline_tail == expected {
        return Ok(());
    }
    Err("assert progress_timeline[-2:] == \"~×\" for no-progress turn timeout".to_string())
}

fn elapsed_progress_marker_due(
    progress: &EvaluatorProgress,
    timeline: &Arc<Mutex<ElapsedProgressTimelineState>>,
    now: Instant,
) -> Result<Option<EvaluatorProgressMarker>, String> {
    let mut timeline = timeline
        .lock()
        .map_err(|_| "check live report progress state poisoned".to_string())?;
    progress.elapsed_marker_due(
        &mut timeline.next_marker_at,
        now,
        PROGRESS_TIMELINE_ELAPSED_MARKER_INTERVAL,
    )
}

fn completion_progress_markers_due(
    progress: &EvaluatorProgress,
    timeline: &Arc<Mutex<ElapsedProgressTimelineState>>,
    now: Instant,
) -> Result<Vec<EvaluatorProgressMarker>, String> {
    let mut timeline = timeline
        .lock()
        .map_err(|_| "check live report progress state poisoned".to_string())?;
    progress.completion_markers_due(
        &mut timeline.next_marker_at,
        now,
        PROGRESS_TIMELINE_ELAPSED_MARKER_INTERVAL,
    )
}

fn wait_for_next_elapsed_marker(timeline: &Arc<Mutex<ElapsedProgressTimelineState>>) -> Duration {
    timeline
        .lock()
        .map(|timeline| {
            timeline
                .next_marker_at
                .saturating_duration_since(Instant::now())
        })
        .unwrap_or(PROGRESS_TIMELINE_ELAPSED_MARKER_INTERVAL)
}

// Results without a live evaluated report still have a progress timeline: the
// complete timeline is the initial marker.
pub(crate) fn write_result_output_without_started_report(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
) -> Result<(), String> {
    if let Some(writer) = result_output.as_mut() {
        let line = render_check_output_record_with_initial_marker_timeline(record);
        write_stdout_record(*writer, line.as_bytes(), "check result")?;
    }
    Ok(())
}

fn render_check_output_record_with_initial_marker_timeline(record: &CheckRecord) -> String {
    let mut output = record.display_id.clone();
    output.push('.');
    output.push_str(&render_check_output_record_status_and_details(record));
    output
}

pub(super) fn render_check_output_record_status_and_details(record: &CheckRecord) -> String {
    // Public rendering receives final normalized records, not just evaluator
    // response payloads. It renders the final CheckRecord as provided;
    // selected-expectation execution asserts ScopeTooNarrow is never a final
    // check result before this public-output boundary.
    // xpec: nO
    debug_assert_ne!(record.error.as_deref(), Some(ERROR_SCOPE_TOO_NARROW));
    let expected = record.expected_answer_text().unwrap_or("");
    let ask_mode = expected.is_empty();
    if record.passed() {
        return (if ask_mode { "\n" } else { " PASS\n" }).to_string();
    }
    let mut output = String::new();
    output.push_str(if ask_mode { "\n" } else { " FAIL\n" });
    if let Some(error) = record.error.as_deref() {
        output.push_str("error: ");
        output.push_str(&escape_check_output_text(error));
        output.push('\n');
        return output;
    }
    if record.to == ExpectationTo::Caller {
        output.push_str("expected: ");
        output.push_str(&escape_check_output_text(
            record.expected_answer_text().unwrap_or(""),
        ));
        output.push('\n');
        return output;
    }
    if record.to == ExpectationTo::Shell {
        for line in record.evidence.lines() {
            output.push_str("│ ");
            output.push_str(line);
            output.push('\n');
        }
        output.push_str(&format!(
            "Command exited with code {} (expected {}).\n",
            record.observed,
            record.expected_answer_text().unwrap_or("")
        ));
        return output;
    }
    if !ask_mode {
        output.push_str(&escape_check_output_text(record.question_text()));
        output.push('\n');
    }
    // The `Diff-from:` line is part of the failed/error result block only when
    // the record came from a Git-backed interrogation with a resolved diff
    // base. Cached records reconstruct the same in-memory abbreviation before
    // reaching this renderer.
    if let (Some(diff_from), Some(diff_from_tree_oid_abbrev)) = (
        record.diff_from.as_deref(),
        record.diff_from_tree_oid_abbrev.as_deref(),
    ) {
        output.push_str("diff-from: ");
        output.push_str(&escape_check_output_text(diff_from_tree_oid_abbrev));
        output.push_str(" (");
        output.push_str(&escape_check_output_text(diff_from));
        output.push_str(")\n");
    }
    if !ask_mode {
        output.push_str("expected: ");
        output.push_str(&escape_check_output_text(expected));
        output.push('\n');
    }
    output.push_str("observed: ");
    output.push_str(&escape_check_output_text(&record.observed));
    output.push('\n');
    output.push_str("evidence: ");
    output.push_str(&escape_check_output_text(&record.evidence));
    output.push('\n');
    if ask_mode {
        if let Some(suggestion) = record.question_scope_suggestion.as_deref() {
            output.push_str("q-scope-suggestion: ");
            output.push_str(&compact_json_string_array(suggestion));
            output.push('\n');
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{
        assert_final_no_progress_turn_timeout_suffix, record_progress_marker,
        start_expectation_report_output, ElapsedProgressTimelineState, SharedCheckOutput,
    };
    use crate::check::core::{CheckRecord, CheckResult, INTERNAL_ERROR_UNPARSABLE};
    use crate::evaluator::EvaluatorProgressMarker;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    #[test] // xpec: Od,sy
    fn turn_timeout_progress_marker_is_allowed_after_idle() {
        let timeline = test_timeline();

        record_progress_marker(&timeline, EvaluatorProgressMarker::Idle).unwrap();
        record_progress_marker(&timeline, EvaluatorProgressMarker::TurnTimeout).unwrap();
        assert_final_no_progress_turn_timeout_suffix(&timeline, &timeout_error_record()).unwrap();
    }

    #[test] // xpec: Od,sy
    fn final_turn_timeout_progress_marker_error_keeps_report_path_alive() {
        let timeline = test_timeline();

        record_progress_marker(&timeline, EvaluatorProgressMarker::TurnTimeout).unwrap();
        let err = assert_final_no_progress_turn_timeout_suffix(&timeline, &timeout_error_record())
            .unwrap_err();

        assert_eq!(
            err,
            "assert progress_timeline[-2:] == \"~×\" for no-progress turn timeout"
        );
    }

    #[test] // xpec: sy,Od
    fn invalid_turn_timeout_timeline_still_writes_final_result() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = SharedCheckOutput::new(Box::new(CapturedOutput {
            bytes: bytes.clone(),
        }));
        let report = start_expectation_report_output(output, "j");
        report.progress().record_turn_timeout();

        let finished = report.finish_with_record(&timeout_error_record());

        assert!(finished.stdout_completion_failed());
        let output = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        assert!(output.contains("j× FAIL\n"));
        assert!(output.contains("error: unparsable\n"));
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

    fn test_timeline() -> Arc<Mutex<ElapsedProgressTimelineState>> {
        Arc::new(Mutex::new(ElapsedProgressTimelineState {
            next_marker_at: Instant::now(),
            progress_timeline_tail: [None, None],
        }))
    }

    fn timeout_error_record() -> CheckRecord {
        CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: 1,
            result: CheckResult::Fail,
            to: crate::config_types::ExpectationTo::Agent,
            question: Some("Does it pass?".to_string()),
            expected_answer: Some("yes".to_string()),
            observed: INTERNAL_ERROR_UNPARSABLE.to_string(),
            error: Some(INTERNAL_ERROR_UNPARSABLE.to_string()),
            evidence: "technical timeout".to_string(),
            scope: vec![".".to_string()],
            question_scope_suggestion: None,
            visible_tree_oid: "visible".to_string(),
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
            id: "11111111111111111111".to_string(),
            display_id: "j".to_string(),
        }
    }
}
