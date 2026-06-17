use super::escape::escape_check_output_text;
use super::shared::{write_stdout_record, SharedCheckOutput};
use crate::check::core::CheckRecord;
use crate::evaluator::{EvaluatorProgress, EvaluatorProgressMarker, EvaluatorProgressSnapshot};
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

pub(crate) struct StartedExpectationReportOutput {
    output: SharedCheckOutput,
    stop: Sender<()>,
    active: Arc<AtomicBool>,
    progress: EvaluatorProgress,
    timeline: Arc<Mutex<ElapsedProgressTimelineState>>,
    worker: Option<JoinHandle<Result<(), String>>>,
    prefix_completed: bool,
}

struct ElapsedProgressTimelineState {
    last_snapshot: EvaluatorProgressSnapshot,
    next_marker_at: Instant,
}

pub(crate) fn start_expectation_report_output(
    output: SharedCheckOutput,
    display_id: &str,
) -> StartedExpectationReportOutput {
    let mut immediate_output = output.clone();
    let prefix_completed = write_stdout_record(
        &mut immediate_output,
        format!("{}.", display_id).as_bytes(),
        "started expectation report prefix",
    )
    .is_ok();

    let (stop, stop_requested) = mpsc::channel();
    let active = Arc::new(AtomicBool::new(true));
    let worker_active = active.clone();
    let progress = EvaluatorProgress::new();
    let worker_progress = progress.clone();
    let initial_snapshot = progress.snapshot();
    let first_elapsed_marker_at = Instant::now() + PROGRESS_TIMELINE_ELAPSED_MARKER_INTERVAL;
    let elapsed_timeline = Arc::new(Mutex::new(ElapsedProgressTimelineState {
        last_snapshot: initial_snapshot,
        next_marker_at: first_elapsed_marker_at,
    }));
    let worker_timeline = elapsed_timeline.clone();
    let mut dot_output = output.clone();
    let worker = thread::spawn(move || loop {
        match stop_requested.recv_timeout(PROGRESS_TIMELINE_ELAPSED_MARKER_INTERVAL) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {
                if !worker_active.load(Ordering::Acquire) {
                    return Ok(());
                }
                let marker = worker_progress.with_snapshot(|snapshot| {
                    let mut timeline = worker_timeline
                        .lock()
                        .map_err(|_| "check live report progress state poisoned")?;
                    let marker = snapshot.marker_since(timeline.last_snapshot);
                    timeline.last_snapshot = snapshot;
                    timeline.next_marker_at =
                        Instant::now() + PROGRESS_TIMELINE_ELAPSED_MARKER_INTERVAL;
                    Ok::<EvaluatorProgressMarker, String>(marker)
                })??;
                write_stdout_record(
                    &mut dot_output,
                    marker.as_str().as_bytes(),
                    "check live report progress marker",
                )?;
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

    pub(crate) fn finish_with_record(mut self, record: &CheckRecord) -> bool {
        // Once the report prefix is visible, final result output has priority
        // over delayed dot-worker cleanup errors.
        let _ = self.stop_dot_worker();
        let result_suffix = if self.prefix_completed {
            let mut result_suffix = String::new();
            if let Some(marker) = self.due_elapsed_progress_marker() {
                result_suffix.push_str(marker.as_str());
            }
            result_suffix.push_str(&render_check_output_record_status_and_details(record));
            result_suffix
        } else {
            render_check_output_record_with_initial_marker_timeline(record)
        };
        let mut output = self.output.clone();
        if write_stdout_record(&mut output, result_suffix.as_bytes(), "check result").is_err() {
            // Human byte sinks can reject all writes; no CLI can force bytes
            // into closed stdout/stderr. The report invariant here is that
            // human-output failure cannot erase the CheckRecord returned for
            // summary accounting, last-result state, or diagnostic logs.
            return write_live_completion_fallback_to_stderr(
                record,
                &result_suffix,
                self.prefix_completed,
            );
        }
        true
    }

    fn stop_dot_worker(&mut self) -> Result<(), String> {
        self.active.store(false, Ordering::Release);
        let _ = self.stop.send(());
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker
            .join()
            .map_err(|_| "check live report dot thread panicked".to_string())?
    }

    fn due_elapsed_progress_marker(&self) -> Option<EvaluatorProgressMarker> {
        let now = Instant::now();
        self.progress
            .with_snapshot(|snapshot| {
                let mut timeline = self.timeline.lock().ok()?;
                if now < timeline.next_marker_at {
                    return None;
                }
                let marker = snapshot.marker_since(timeline.last_snapshot);
                timeline.last_snapshot = snapshot;
                timeline.next_marker_at = now + PROGRESS_TIMELINE_ELAPSED_MARKER_INTERVAL;
                Some(marker)
            })
            .ok()
            .flatten()
    }
}

fn write_live_completion_fallback_to_stderr(
    record: &CheckRecord,
    completion: &str,
    prefix_completed: bool,
) -> bool {
    let fallback = if prefix_completed {
        format!("{}{}", record.display_id, completion)
    } else {
        completion.to_string()
    };
    // Best-effort human-output fallback only. A closed stderr must not panic
    // before the caller can return the CheckRecord for report accounting.
    let mut stderr = std::io::stderr();
    stderr
        .write_all(fallback.as_bytes())
        .and_then(|_| stderr.flush())
        .is_ok()
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

pub(crate) fn write_cached_non_pass_output(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
) -> Result<(), String> {
    // Cached non-passes are displayed issue reports. They are not evaluated in
    // this run, so their complete progress timeline is the initial marker.
    debug_assert!(!record.passed());
    if let Some(writer) = result_output.as_mut() {
        let line = render_check_output_record_with_initial_marker_timeline(record);
        write_stdout_record(*writer, line.as_bytes(), "cached check result")?;
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
    if record.passed() {
        return " OK\n".to_string();
    }
    let is_error = record.requires_human_review();
    let status = if is_error { "ERROR" } else { "FAILED" };
    let mut output = String::new();
    output.push_str(&format!(" {}\n", status));
    output.push_str(&escape_check_output_text(record.question_text()));
    output.push('\n');
    if is_error {
        output.push_str("Error: ");
        let error = record
            .human_review_reason()
            .expect("error records must expose an error value");
        output.push_str(&escape_check_output_text(error));
        output.push('\n');
    } else {
        output.push_str("Expected: ");
        output.push_str(&escape_check_output_text(
            record.expected_answer_text().unwrap_or(""),
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
        if let Some(suggestion) = record.question_scope_suggestion.as_deref() {
            output.push_str("Suggested q-scope: ");
            output.push_str(&compact_json_string_array(suggestion));
            output.push('\n');
        }
    }
    output
}
