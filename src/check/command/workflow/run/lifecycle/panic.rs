use super::super::evaluation::prepare_default_failure_output;
use super::CheckCommandInspection;
use crate::check::command::workflow::failure::{
    write_check_failure_feedback, write_unconditional_check_trailer_and_feedback_for_report,
    CheckFailureOutput, CheckPublicOutputProgress,
};
use crate::check::command::TokenUsageSummary;
use crate::check::core::CheckRunReport;
use crate::check::interrogation::{
    write_check_lifecycle_finish_event, write_check_lifecycle_start_event,
};
use crate::logs::DiagnosticLogWriter;
use std::any::Any;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::path::Path;

const PANICKED_CHECK_ERROR: &str = "check panicked";

pub(super) struct PanickedCheckContext<'a> {
    pub(super) root: &'a Path,
    pub(super) in_place: bool,
    pub(super) public_output_progress: CheckPublicOutputProgress,
    pub(super) failure_output: &'a mut CheckFailureOutput,
    pub(super) progress_report: &'a CheckRunReport,
    pub(super) panic_token_usage: TokenUsageSummary,
    pub(super) diagnostic_log: &'a mut DiagnosticLogWriter,
    pub(super) inspection: &'a mut CheckCommandInspection,
}

pub(super) fn resume_panicked_check(
    payload: Box<dyn Any + Send>,
    context: PanickedCheckContext<'_>,
) -> ! {
    let PanickedCheckContext {
        root,
        in_place,
        public_output_progress,
        failure_output,
        progress_report,
        panic_token_usage,
        diagnostic_log,
        inspection,
    } = context;
    // [w] Evaluation assertions are allowed to panic, but the command's
    // `finally` effects still run. Preserve the original panic after
    // attempting any missing public trailer or feedback effects, lifecycle
    // finish, and deferred runtime-log writes. Secondary panics in fallback
    // rendering are contained.
    if public_output_progress.needs_trailer() || public_output_progress.needs_feedback() {
        *failure_output = attempt_panicked_check_public_output(
            *failure_output,
            || prepare_default_failure_output(root, *failure_output, in_place, inspection),
            |prepared_output| {
                if public_output_progress.needs_trailer() {
                    let _ = write_unconditional_check_trailer_and_feedback_for_report(
                        prepared_output,
                        progress_report,
                        panic_token_usage,
                    );
                } else if public_output_progress.needs_feedback() {
                    let _ = write_check_failure_feedback(prepared_output, progress_report);
                }
            },
        );
    }
    finish_panicked_check_lifecycle(diagnostic_log, failure_output.lifecycle_started());
    let _ = diagnostic_log.finish_deferred_writes();
    resume_unwind(payload)
}

fn attempt_panicked_check_public_output(
    original_output: CheckFailureOutput,
    prepare: impl FnOnce() -> CheckFailureOutput,
    write: impl FnOnce(CheckFailureOutput),
) -> CheckFailureOutput {
    // [w] Preparation and output are independent panic boundaries. If default
    // tree context cannot be prepared, token usage and summary still run from
    // the original progress state before any feedback assertion can fail.
    let prepared_output = catch_unwind(AssertUnwindSafe(prepare)).unwrap_or(original_output);
    let _ = catch_unwind(AssertUnwindSafe(|| write(prepared_output)));
    prepared_output
}

fn finish_panicked_check_lifecycle(
    diagnostic_log: &mut DiagnosticLogWriter,
    lifecycle_started: bool,
) {
    // [w] A panic before candidate collection still gets a complete empty
    // lifecycle bracket. A later panic reuses the recorded start and appends
    // exactly one failed finish before deferred log errors are collected.
    if !lifecycle_started {
        let _ = write_check_lifecycle_start_event(diagnostic_log, None, Vec::new());
    }
    let _ = write_check_lifecycle_finish_event(diagnostic_log, false, Some(PANICKED_CHECK_ERROR));
}
