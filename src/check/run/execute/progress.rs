use crate::check::command::output::{
    start_expectation_report_output, SharedCheckOutput, StartedExpectationReportOutput,
};
use crate::check::core::{CheckRecord, SelectedExpectation};
use crate::evaluator::EvaluatorProgress;
use std::path::Path;

pub(super) struct LiveExpectationReport {
    output: StartedExpectationReportOutput,
}

pub(super) fn start_live_expectation_report(
    _state_root: Option<&Path>,
    output: &SharedCheckOutput,
    expectation: &SelectedExpectation,
) -> Result<LiveExpectationReport, String> {
    // The flushed `<short ID>.` prefix is the started report. Invocation-local
    // report state stays in memory; if completion stdout later fails, finish
    // emits an emergency completed report on stderr and still returns the
    // CheckRecord through the in-memory CheckRunReport. The caller contract is
    // that every command-controlled fallible or interrupted post-start path
    // calls `finish_public_output_or_keep_state_report` with either the normal
    // result or an ERROR record; there is intentionally no separate cancel
    // operation for a started expectation.
    Ok(LiveExpectationReport {
        output: start_expectation_report_output(output.clone(), &expectation.display_id),
    })
}

impl LiveExpectationReport {
    pub(super) fn progress(&self) -> EvaluatorProgress {
        self.output.progress()
    }

    pub(super) fn finish_public_output_or_keep_state_report(
        self,
        record: &CheckRecord,
    ) -> Result<(), String> {
        let finished = self.output.finish_with_record(record);
        if finished.backup_report_needed() {
            // If stdout accepted the short-ID prefix but cannot receive the
            // completed result, stderr is the emergency completed report for
            // the affected expectation. Stderr is not the ownership boundary
            // for the record: the CheckRecord still flows to CheckRunReport
            // after this function returns.
            eprintln!("{}", output_only_backup_report_line(record));
        }
        Ok(())
    }
}

fn output_only_backup_report_line(record: &CheckRecord) -> String {
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
