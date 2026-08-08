use super::output::{write_unconditional_check_trailer_and_feedback, CheckFailureOutput};
use crate::check::command::output::CheckFeedbackContext;
use crate::check::command::workflow::failure::CheckPublicOutputProgress;
use crate::check::command::{finish_check_report, CheckReportFinishContext};
use crate::check::core::CheckRunReport;
use crate::check::interrogation::{
    write_check_lifecycle_finish_event, write_check_lifecycle_start_event,
};
use crate::cli::{write_command_error_line, CommandError, ReportedCommandFailure};
use crate::logs::DiagnosticLogWriter;
use std::io::Write;

pub(in crate::check::command::workflow) struct CheckErrorReportFinish<'b> {
    pub(in crate::check::command::workflow) diagnostic_log: &'b mut DiagnosticLogWriter,
    pub(in crate::check::command::workflow) result_output: &'b mut dyn Write,
    pub(in crate::check::command::workflow) report: &'b CheckRunReport,
    pub(in crate::check::command::workflow) error: String,
    pub(in crate::check::command::workflow) feedback_context: Option<CheckFeedbackContext>,
    pub(in crate::check::command::workflow) failure_history_feedback:
        Option<&'b crate::xpec_state::FailureHistoryFeedback>,
}

pub(in crate::check::command::workflow) fn finish_check_error_report(
    context: CheckErrorReportFinish<'_>,
) -> Result<(), CommandError> {
    let error = context.error;
    // [kK] This path follows a failed `write_check_trailer`, which already
    // attempted both unconditional trailer parts independently. Finish the
    // remaining feedback and lifecycle-log parts without duplicating either
    // trailer line.
    finish_check_report(
        CheckReportFinishContext {
            diagnostic_log: Some(context.diagnostic_log),
            result_output: context.result_output,
            feedback_context: context.feedback_context,
            failure_history_feedback: context.failure_history_feedback,
        },
        context.report,
        Some(&error),
    )?;
    Err(error.into())
}

pub(in crate::check::command::workflow) fn fail_check_before_selection(
    diagnostic_log: &mut DiagnosticLogWriter,
    public_output_progress: &mut CheckPublicOutputProgress,
    output: &mut CheckFailureOutput,
    err: String,
) -> Result<(), CommandError> {
    // [kK] Failed runtime-log storage must not suppress the public `finally`
    // output. Preserve the start-log failure in the command error, then
    // continue through the common trailer and finish-log path.
    let err = match write_check_lifecycle_start_event(diagnostic_log, None, Vec::new()) {
        Ok(()) => {
            output.mark_lifecycle_started();
            err
        }
        Err(start_error) => {
            format!("{err}; also failed to write check lifecycle start event: {start_error}")
        }
    };
    finish_check_failure(diagnostic_log, public_output_progress, *output, err)
}

#[derive(Clone, Copy)]
pub(in crate::check::command::workflow) enum SelectionBoundary {
    Before,
    After,
}

pub(in crate::check::command::workflow) fn or_fail_at_selection_boundary<T, E: ToString>(
    result: Result<T, E>,
    boundary: SelectionBoundary,
    diagnostic_log: &mut DiagnosticLogWriter,
    public_output_progress: &mut CheckPublicOutputProgress,
    output: &mut CheckFailureOutput,
) -> Result<T, CommandError> {
    or_finalize(result, |err| {
        let error = err.to_string();
        match boundary {
            SelectionBoundary::Before => {
                fail_check_before_selection(diagnostic_log, public_output_progress, output, error)
            }
            SelectionBoundary::After => {
                fail_check_after_selection(diagnostic_log, public_output_progress, output, error)
            }
        }
    })
}

pub(in crate::check::command::workflow) fn start_check_with_candidates_or_fail(
    diagnostic_log: &mut DiagnosticLogWriter,
    candidate_ids: Vec<String>,
    public_output_progress: &mut CheckPublicOutputProgress,
    output: &mut CheckFailureOutput,
) -> Result<(), CommandError> {
    // [kK,gN] Candidate resolution establishes the command lifecycle boundary;
    // cache filtering establishes the canonical Selected set later. If
    // start-event storage itself fails, the command still traverses the same
    // public trailer and best-effort finish-event path as every later failure.
    match write_check_lifecycle_start_event(diagnostic_log, None, candidate_ids) {
        Ok(()) => {
            output.mark_lifecycle_started();
            Ok(())
        }
        Err(err) => fail_check_after_selection(
            diagnostic_log,
            public_output_progress,
            output,
            format!("failed to write check lifecycle start event: {err}"),
        ),
    }
}

pub(in crate::check::command::workflow) fn fail_check_after_selection(
    diagnostic_log: &mut DiagnosticLogWriter,
    public_output_progress: &mut CheckPublicOutputProgress,
    output: &mut CheckFailureOutput,
    err: String,
) -> Result<(), CommandError> {
    // [2Z,kK] Preserve the collected pending count for the summary. Feedback
    // remains the reported-error action until the run reaches evaluation
    // readiness; after that boundary, failed or pending outcomes take priority.
    // Earlier failures reach this finisher with as much default tree context as
    // the failing repository state allowed preparation to establish.
    finish_check_failure(diagnostic_log, public_output_progress, *output, err)
}

pub(in crate::check::command::workflow) fn fail_check_before_lifecycle(
    output: CheckFailureOutput,
    err: String,
) -> Result<(), CommandError> {
    write_check_failure_public_effects(None, output, &err)?;
    Err(CommandError::Reported(ReportedCommandFailure::Check))
}

pub(in crate::check::command::workflow) fn or_finalize<T, E>(
    result: Result<T, E>,
    finalize: impl FnOnce(E) -> Result<(), CommandError>,
) -> Result<T, CommandError> {
    match result {
        Ok(value) => Ok(value),
        Err(source_error) => match finalize(source_error) {
            Err(error) => Err(error),
            Ok(()) => unreachable!("check failure finalization must return an error"),
        },
    }
}

fn finish_check_failure(
    diagnostic_log: &mut DiagnosticLogWriter,
    public_output_progress: &mut CheckPublicOutputProgress,
    output: CheckFailureOutput,
    err: String,
) -> Result<(), CommandError> {
    let public_result =
        write_check_failure_public_effects(Some(public_output_progress), output, &err);
    // [kK] The public trailer and lifecycle logging are independent failure
    // effects. Attempt the log even when public output fails, and never let a
    // logging failure suppress the unconditional check trailer.
    let finish_result =
        write_check_error_finish_event(diagnostic_log, &err).map_err(CommandError::from);
    combine_failure_effect_results([public_result, finish_result])?;
    Err(CommandError::Reported(ReportedCommandFailure::Check))
}

fn write_check_failure_public_effects(
    public_output_progress: Option<&mut CheckPublicOutputProgress>,
    output: CheckFailureOutput,
    err: &str,
) -> Result<(), CommandError> {
    attempt_failure_diagnostic_then_public_finally(
        public_output_progress,
        || {
            write_command_error_line(&CommandError::from(err.to_owned()))
                .map_err(CommandError::from)
        },
        || write_unconditional_check_trailer_and_feedback(output),
    )
}

fn attempt_failure_diagnostic_then_public_finally(
    public_output_progress: Option<&mut CheckPublicOutputProgress>,
    write_diagnostic: impl FnOnce() -> Result<(), CommandError>,
    write_protected_finally: impl FnOnce() -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    // [1h,D8] Once a check-specific failure is known, its diagnostic is
    // eligible immediately. Flush it before unrelated `finally` effects, then
    // return a sentinel so the outer command boundary does not print it twice.
    let diagnostic_result = write_diagnostic();
    // [2Z,kK] This check-only `finally` path always attempts token usage, a
    // summary, and eligible feedback. Collected xpecs without results remain
    // pending in the summary. Feedback uses the reported-error action until the
    // run is ready for evaluation, then preserves failed or pending actions.
    // Before a collection attempt, it uses the continuation action when tree
    // context is available. A failed collection uses the reported-error
    // action; without tree context, any command error uses the generic action.
    // Transfer ownership only after the diagnostic attempt returns. A
    // diagnostic panic therefore leaves every finally effect eligible for the
    // outer panic fallback. The transferred operation protects token usage,
    // summary, and feedback independently before it resumes any panic.
    if let Some(progress) = public_output_progress {
        progress.mark_all_attempted();
    }
    let trailer_result = write_protected_finally();
    combine_failure_effect_results([diagnostic_result, trailer_result])
}

pub(in crate::check::command::workflow) fn combine_failure_effect_results<const N: usize>(
    results: [Result<(), CommandError>; N],
) -> Result<(), CommandError> {
    let mut errors = results.into_iter().filter_map(Result::err);
    let Some(first) = errors.next() else {
        return Ok(());
    };
    let Some(second) = errors.next() else {
        return Err(first);
    };
    let mut message = first.to_string();
    message.push_str("; also ");
    message.push_str(&second.to_string());
    for error in errors {
        message.push_str("; also ");
        message.push_str(&error.to_string());
    }
    Err(CommandError::from(message))
}

pub(super) fn write_check_error_finish_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    err: &str,
) -> Result<(), String> {
    write_check_lifecycle_finish_event(diagnostic_log, false, Some(err))
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: kK
    fn diagnostic_panic_leaves_public_finally_effects_for_outer_fallback() {
        let mut progress = CheckPublicOutputProgress::default();
        let mut protected_finally_started = false;

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = attempt_failure_diagnostic_then_public_finally(
                Some(&mut progress),
                || -> Result<(), CommandError> { panic!("diagnostic panicked") },
                || {
                    protected_finally_started = true;
                    Ok(())
                },
            );
        }))
        .unwrap_err();

        assert!(!protected_finally_started);
        assert!(progress.needs_trailer());
        assert!(progress.needs_feedback());
        assert_eq!(panic.downcast_ref::<&str>(), Some(&"diagnostic panicked"));
    }
}
