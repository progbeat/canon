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
        if finished.stdout_completion_failed() {
            // If stdout accepted the short-ID prefix but cannot receive the
            // completed result, stderr gets an emergency public-output notice.
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
