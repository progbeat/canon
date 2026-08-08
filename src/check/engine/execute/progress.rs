use crate::check::command::output::{
    publish_expectation_report, LiveProgressOutput, SharedCheckOutput,
};
use crate::check::core::{CheckRecord, ResolvedExpectation};
use crate::evaluator::EvaluatorProgress;
use crate::output::write_stderr_line;
use std::path::Path;

pub(super) struct LiveExpectationReport {
    output: LiveProgressOutput,
}

pub(super) fn start_live_expectation_report(
    _state_root: Option<&Path>,
    output: &SharedCheckOutput,
    expectation: &ResolvedExpectation,
) -> Result<LiveExpectationReport, String> {
    // Publishing the flushed `<short ID>` creates the public expectation
    // report before evaluator work. This module owns that public output; the
    // structured `CheckRunReport` separately owns the aggregate run record.
    // Every command-controlled fallible or interrupted post-publication path
    // appends either the normal result or an ERROR result, then continues into
    // the structured run record.
    Ok(LiveExpectationReport {
        output: publish_expectation_report(output.clone(), &expectation.display_id),
    })
}

impl LiveExpectationReport {
    pub(super) fn progress(&self) -> EvaluatorProgress {
        self.output.progress()
    }

    pub(super) fn append_result_before_structured_record(self, record: &CheckRecord) {
        let outcome = self.output.append_result(record);
        if outcome.needs_stderr_result_notice() {
            // If stdout cannot receive the result details, stderr gets an
            // emergency result notice. When stdout already accepted the short
            // ID, something has already been reported for this expectation
            // even if this best-effort notice also encounters an I/O failure.
            // The notice does not own reporting: the CheckRecord still flows to
            // the structured report after this function returns.
            let _ = write_stderr_line(&emergency_result_notice_line(record));
        }
    }
}

fn emergency_result_notice_line(record: &CheckRecord) -> String {
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
