use crate::check::command::output::{
    start_expectation_report_output, SharedCheckOutput, StartedExpectationReportOutput,
};
use crate::check::core::{CheckRecord, ResolvedExpectation};
use crate::evaluator::EvaluatorProgress;
use crate::output::write_stderr_line;
use std::path::Path;

pub(super) struct LiveExpectationReport {
    output: StartedExpectationReportOutput,
}

pub(super) fn start_live_expectation_report(
    _state_root: Option<&Path>,
    output: &SharedCheckOutput,
    expectation: &ResolvedExpectation,
) -> Result<LiveExpectationReport, String> {
    // The flushed `<short ID>.` prefix is already public output for this
    // expectation: it starts the report before evaluator work. This module owns
    // public output only; the structured `CheckRunReport` remains the report
    // owner. Every command-controlled fallible or interrupted post-start path
    // calls this component with either the normal result or an ERROR record,
    // then continues into the structured report.
    Ok(LiveExpectationReport {
        output: start_expectation_report_output(output.clone(), &expectation.display_id),
    })
}

impl LiveExpectationReport {
    pub(super) fn progress(&self) -> EvaluatorProgress {
        self.output.progress()
    }

    pub(super) fn finish_public_output_before_structured_report(self, record: &CheckRecord) {
        let finished = self.output.finish_with_record(record);
        // xpec: sy
        assert!(
            !finished.short_id_was_printed() || finished.anything_was_reported(),
            "a printed expectation short ID must itself remain a public report"
        );
        if finished.needs_stderr_completion_notice() {
            // If stdout cannot receive the completed result, stderr gets an
            // emergency completion notice. When stdout already accepted the
            // short ID, that prefix remains visible as the started public
            // report even if this best-effort notice also encounters an I/O
            // failure.
            // The notice does not own reporting: the CheckRecord still flows to
            // the structured report after this function returns.
            let _ = write_stderr_line(&emergency_completion_notice_line(record));
        }
    }
}

fn emergency_completion_notice_line(record: &CheckRecord) -> String {
    let status = if record.requires_human_review() {
        "error"
    } else if record.passed() {
        "pass"
    } else {
        "fail"
    };
    format!(
        "canon check: completed report for expectation {}: {}",
        record.display_id, status
    )
}
