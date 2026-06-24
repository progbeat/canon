use super::escape::escape_check_output_text;
use super::shared::{write_stdout_record, SharedCheckOutput};
use crate::check::core::{CheckRecord, ERROR_SCOPE_TOO_NARROW};
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
    backup_report_needed: bool,
}

impl FinishedExpectationReportOutput {
    fn completed_report() -> FinishedExpectationReportOutput {
        FinishedExpectationReportOutput {
            backup_report_needed: false,
        }
    }

    fn backup_report() -> FinishedExpectationReportOutput {
        FinishedExpectationReportOutput {
            backup_report_needed: true,
        }
    }

    pub(crate) fn backup_report_needed(&self) -> bool {
        self.backup_report_needed
    }
}

struct ElapsedProgressTimelineState {
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
    let first_elapsed_marker_at = Instant::now() + PROGRESS_TIMELINE_ELAPSED_MARKER_INTERVAL;
    let elapsed_timeline = Arc::new(Mutex::new(ElapsedProgressTimelineState {
        next_marker_at: first_elapsed_marker_at,
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
                let markers = elapsed_progress_markers_due(
                    &worker_progress,
                    &worker_timeline,
                    Instant::now(),
                )?;
                for marker in markers {
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
        if self.prefix_completed {
            let markers = match self.due_elapsed_progress_markers() {
                Ok(markers) => markers,
                Err(_) => return FinishedExpectationReportOutput::backup_report(),
            };
            for marker in markers {
                if write_stdout_record(
                    &mut output,
                    marker.as_str().as_bytes(),
                    "check live report progress marker",
                )
                .is_err()
                {
                    return FinishedExpectationReportOutput::backup_report();
                }
            }
        }
        let result_suffix = if self.prefix_completed {
            render_check_output_record_status_and_details(record)
        } else {
            render_check_output_record_with_initial_marker_timeline(record)
        };
        if write_stdout_record(&mut output, result_suffix.as_bytes(), "check result").is_err() {
            return FinishedExpectationReportOutput::backup_report();
        }
        FinishedExpectationReportOutput::completed_report()
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
        elapsed_progress_markers_due(&self.progress, &self.timeline, Instant::now())
    }
}

fn elapsed_progress_markers_due(
    progress: &EvaluatorProgress,
    timeline: &Arc<Mutex<ElapsedProgressTimelineState>>,
    now: Instant,
) -> Result<Vec<EvaluatorProgressMarker>, String> {
    let mut timeline = timeline
        .lock()
        .map_err(|_| "check live report progress state poisoned".to_string())?;
    progress.elapsed_markers_due(
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
    assert_ne!(
        record.human_review_reason(),
        Some(ERROR_SCOPE_TOO_NARROW),
        "user-visible final check results must not expose ScopeTooNarrow"
    );
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
        // The optional Suggested q-scope line is emitted only while an
        // invocation-local evaluator suggestion is still available. Cached
        // records reconstructed from persistent last-result state omit it.
        if let Some(suggestion) = record.question_scope_suggestion.as_deref() {
            output.push_str("Suggested q-scope: ");
            output.push_str(&compact_json_string_array(suggestion));
            output.push('\n');
        }
    }
    output
}
