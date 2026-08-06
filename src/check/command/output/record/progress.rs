mod timeline;

use timeline::ElapsedProgressTimeline;

use super::{
    render_check_output_record_status_and_details, render_check_output_record_with_timeline,
    ExpectationReportWriteOutcome,
};
use crate::check::command::output::shared::SharedCheckOutput;
use crate::check::core::CheckRecord;
use crate::evaluator::{
    EvaluatorProgress, EvaluatorProgressMarker, PROGRESS_TIMELINE_MARKER_INTERVAL,
};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Instant;

// Progress timeline ownership:
// - `publish_expectation_report` writes and flushes `<short ID>` before
//   evaluator work starts; the short ID is the public expectation report, not
//   a pre-report prefix or timeline marker.
// - `start_query_report_output` writes and flushes the temporary xpec's short
//   ID before evaluator work starts.
// - after a short ID is public, the progress worker writes and flushes elapsed
//   markers every minute while evaluator work is still active. If the initial
//   fragment was not fully accepted, it records those markers without
//   emitting orphaned bytes. A fully accepted short ID may still receive its
//   markers when the initial flush failed.
// - when evaluation is ready to report, the due final marker is written before
//   the result suffix or response block. It is emitted even when the final
//   interval is 0 seconds.
// - a finished query writes the newline after those markers before the query
//   response, so its stdout starts with the temporary short ID and timeline.
// Event-to-marker priority and symbols live in `src/evaluator/progress.rs`.
pub(crate) struct LiveProgressOutput {
    output: SharedCheckOutput,
    stop: Sender<()>,
    active: Arc<AtomicBool>,
    progress: EvaluatorProgress,
    timeline: Arc<ElapsedProgressTimeline>,
    worker: Option<JoinHandle<Result<(), String>>>,
    short_id_report_was_printed: bool,
    initial_output_completed: bool,
}

pub(crate) fn publish_expectation_report(
    output: SharedCheckOutput,
    display_id: &str,
) -> LiveProgressOutput {
    start_live_progress_output(output, display_id, "public expectation report short ID")
}

pub(crate) fn start_query_report_output(
    output: SharedCheckOutput,
    display_id: &str,
) -> LiveProgressOutput {
    start_live_progress_output(output, display_id, "public query report short ID")
}

fn start_live_progress_output(
    output: SharedCheckOutput,
    prefix: &str,
    description: &str,
) -> LiveProgressOutput {
    let prefix_output = output.write_fragment(prefix.as_bytes(), description);
    let short_id_report_was_printed =
        !prefix.is_empty() && prefix_output.entire_fragment_was_written();
    let initial_output_completed = prefix_output.completed();

    let (stop, stop_requested) = mpsc::channel();
    let active = Arc::new(AtomicBool::new(true));
    let worker_active = active.clone();
    let progress = EvaluatorProgress::new();
    let worker_progress = progress.clone();
    let first_elapsed_marker_at = Instant::now() + PROGRESS_TIMELINE_MARKER_INTERVAL;
    let elapsed_timeline = Arc::new(ElapsedProgressTimeline::new(first_elapsed_marker_at));
    let worker_timeline = elapsed_timeline.clone();
    let progress_output = output.clone();
    let stream_elapsed_markers = short_id_report_was_printed;
    let worker = thread::spawn(move || loop {
        match stop_requested.recv_timeout(worker_timeline.wait_for_next_elapsed_marker()) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {
                if !worker_active.load(Ordering::Acquire) {
                    return Ok(());
                }
                let marker = worker_timeline
                    .take_elapsed_progress_marker_due(&worker_progress, Instant::now())?;
                if let Some(marker) = marker {
                    if stream_elapsed_markers {
                        progress_output
                            .write_fragment(
                                marker.as_str().as_bytes(),
                                "check live report progress marker",
                            )
                            .into_result()?;
                    }
                }
            }
        }
    });

    LiveProgressOutput {
        output,
        stop,
        active,
        progress,
        timeline: elapsed_timeline,
        worker: Some(worker),
        short_id_report_was_printed,
        initial_output_completed,
    }
}

impl LiveProgressOutput {
    pub(crate) fn progress(&self) -> EvaluatorProgress {
        self.progress.clone()
    }

    pub(crate) fn append_result(mut self, record: &CheckRecord) -> ExpectationReportWriteOutcome {
        // Once the short-ID report is public, appending its result has priority
        // over delayed progress-worker cleanup errors.
        let ready_to_report = self.timeline.mark_ready_to_report();
        // [2gZ] This join is the ordering barrier between an elapsed marker
        // already taken by the worker and every final marker written below.
        let worker_result = self.stop_and_join_progress_worker();
        let mut output = self.output.clone();
        let mut result_append_problem = worker_result.is_err();
        let final_markers = ready_to_report.and_then(|()| self.record_due_final_markers());
        match final_markers {
            Ok(markers) => {
                if self.short_id_report_was_printed
                    && Self::write_progress_markers(&mut output, &markers).is_err()
                {
                    result_append_problem = true;
                }
            }
            Err(_) => result_append_problem = true,
        }
        let result_suffix = if self.short_id_report_was_printed {
            render_check_output_record_status_and_details(record)
        } else {
            match self.timeline.rendered_progress_timeline() {
                Ok(timeline) if !timeline.is_empty() => {
                    render_check_output_record_with_timeline(record, &timeline)
                }
                Ok(_) | Err(_) => {
                    result_append_problem = true;
                    render_check_output_record_with_timeline(record, ".")
                }
            }
        };
        let result_output = output.write_fragment(result_suffix.as_bytes(), "check result");
        // A fully written short ID is already the public report for this
        // expectation. The result suffix adds details when possible; losing
        // the flush or suffix cannot make the bytes already written cease to
        // be a report.
        ExpectationReportWriteOutcome::new(
            self.short_id_report_was_printed,
            result_output.entire_fragment_was_written(),
            result_append_problem || !result_output.completed(),
        )
    }

    pub(crate) fn finish_with_query_output(mut self, query_output: &str) -> Result<(), String> {
        let ready_to_report = self.timeline.mark_ready_to_report();
        // [2gZ] Query output uses the same worker-before-final ordering barrier.
        let worker_result = self.stop_and_join_progress_worker();
        // [2gZ] Once evaluation is ready, complete the timeline even when an
        // earlier stdout flush or worker cleanup prevents publishing the
        // query response. Output failures must not erase its final minute.
        let final_markers = ready_to_report.and_then(|()| self.record_due_final_markers());
        worker_result?;
        let final_markers = final_markers?;
        let mut output = self.output.clone();
        if self.short_id_report_was_printed {
            Self::write_progress_markers(&mut output, &final_markers)?;
        }
        if !self.initial_output_completed {
            return Err("failed to flush started query report timeline to stdout".to_string());
        }
        output
            .write_fragment(b"\n", "query progress timeline newline")
            .into_result()?;
        output
            .write_fragment(query_output.as_bytes(), "query result")
            .into_result()
    }

    fn stop_and_join_progress_worker(&mut self) -> Result<(), String> {
        self.active.store(false, Ordering::Release);
        let _ = self.stop.send(());
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker
            .join()
            .map_err(|_| "check live report progress thread panicked".to_string())?
    }

    fn record_due_final_markers(&self) -> Result<Vec<EvaluatorProgressMarker>, String> {
        let markers = self.timeline.due_final_progress_markers(&self.progress)?;
        for marker in &markers {
            self.timeline.record_progress_marker(*marker)?;
        }
        // [2gZ,Od] Every completion path goes through this operation, so the
        // canonical terminal-timeout assertion cannot be skipped by a caller.
        self.timeline.assert_turn_timeout_has_idle_suffix()?;
        Ok(markers)
    }

    fn write_progress_markers(
        output: &mut SharedCheckOutput,
        markers: &[EvaluatorProgressMarker],
    ) -> Result<(), String> {
        for marker in markers {
            output
                .write_fragment(
                    marker.as_str().as_bytes(),
                    "check live report progress marker",
                )
                .into_result()?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn record_elapsed_marker_for_test(
        &self,
        marker: EvaluatorProgressMarker,
    ) -> Result<(), String> {
        self.timeline.record_progress_marker(marker)
    }
}

#[cfg(test)]
mod tests {
    use super::{start_query_report_output, SharedCheckOutput};
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    #[test] // xpec: 2gZ
    fn failed_initial_query_flush_still_emits_the_final_marker() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = SharedCheckOutput::new(Box::new(FirstFlushFailure {
            bytes: bytes.clone(),
            first_flush: true,
        }));
        let report = start_query_report_output(output, "q");

        let result = report.finish_with_query_output("observed: yes\n");

        assert!(result.is_err());
        assert_eq!(*bytes.lock().unwrap(), b"q.");
    }

    struct FirstFlushFailure {
        bytes: Arc<Mutex<Vec<u8>>>,
        first_flush: bool,
    }

    impl Write for FirstFlushFailure {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.first_flush {
                self.first_flush = false;
                return Err(io::Error::other("stdout flush failed"));
            }
            Ok(())
        }
    }
}
