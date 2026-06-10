use super::escape::escape_check_output_text;
use super::shared::{write_stdout_record, SharedCheckOutput};
use crate::check::core::CheckRecord;
use crate::json_util::compact_json_string_array;
use std::io::Write;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const PROGRESS_DOT_INTERVAL: Duration = Duration::from_secs(60);

pub(crate) struct LiveCheckProgressOutput {
    output: SharedCheckOutput,
    stop: Sender<()>,
    active: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<(), String>>>,
}

pub(crate) fn start_live_check_progress_output(
    output: SharedCheckOutput,
    display_id: &str,
) -> Result<LiveCheckProgressOutput, String> {
    // Evaluated expectations call this before evaluator work starts. The first
    // dot is written and flushed immediately; the worker appends later dots
    // while the evaluator is still running.
    let mut immediate_output = output.clone();
    write_stdout_record(
        &mut immediate_output,
        format!("{}.", display_id).as_bytes(),
        "check progress prefix",
    )?;

    let (stop, stop_requested) = mpsc::channel();
    let active = Arc::new(AtomicBool::new(true));
    let worker_active = active.clone();
    let mut progress_output = output.clone();
    let worker = thread::spawn(move || loop {
        match stop_requested.recv_timeout(PROGRESS_DOT_INTERVAL) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {
                if !worker_active.load(Ordering::Acquire) {
                    return Ok(());
                }
                write_stdout_record(&mut progress_output, b".", "check progress dot")?;
            }
        }
    });

    Ok(LiveCheckProgressOutput {
        output,
        stop,
        active,
        worker: Some(worker),
    })
}

impl LiveCheckProgressOutput {
    pub(crate) fn finish_with_record(mut self, record: &CheckRecord) -> Result<(), String> {
        // Once the progress prefix is visible, final result output has
        // priority over delayed progress-dot cleanup errors.
        let _ = self.stop_progress_worker();
        let completion = render_check_output_record_completion(record);
        let mut output = self.output.clone();
        if let Err(stdout_err) =
            write_stdout_record(&mut output, completion.as_bytes(), "check result")
        {
            // Best-effort fallback only: the in-memory run record must still
            // be returned so the expectation remains reported by the run.
            let _ = write_live_completion_fallback_to_stderr(record, &completion, &stdout_err);
        }
        Ok(())
    }

    fn stop_progress_worker(&mut self) -> Result<(), String> {
        self.active.store(false, Ordering::Release);
        let _ = self.stop.send(());
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker
            .join()
            .map_err(|_| "check progress thread panicked".to_string())?
    }
}

fn write_live_completion_fallback_to_stderr(
    record: &CheckRecord,
    completion: &str,
    stdout_err: &str,
) -> Result<(), String> {
    let fallback = format!("{}{}", record.display_id, completion);
    let mut stderr = std::io::stderr();
    stderr
        .write_all(fallback.as_bytes())
        .and_then(|_| stderr.flush())
        .map_err(|stderr_err| {
            format!(
                "{}; failed to write fallback check result to stderr: {}",
                stdout_err, stderr_err
            )
        })
}

// No-progress callers still write the documented minimum progress prefix. They
// do not represent an evaluator currently running, so a single dot is enough.
pub(crate) fn write_result_output_without_live_progress(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
) -> Result<(), String> {
    if let Some(writer) = result_output.as_mut() {
        let line = render_check_output_record_with_progress_dot(record);
        write_stdout_record(*writer, line.as_bytes(), "check result")?;
    }
    Ok(())
}

pub(crate) fn write_cached_non_pass_output(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
) -> Result<(), String> {
    debug_assert!(!record.passed());
    if let Some(writer) = result_output.as_mut() {
        let line = render_check_output_record_with_progress_dot(record);
        write_stdout_record(*writer, line.as_bytes(), "cached check result")?;
    }
    Ok(())
}

fn render_check_output_record_with_progress_dot(record: &CheckRecord) -> String {
    let mut output = record.display_id.clone();
    output.push('.');
    output.push_str(&render_check_output_record_completion(record));
    output
}

pub(super) fn render_check_output_record_completion(record: &CheckRecord) -> String {
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
