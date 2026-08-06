use super::PreparedCheckRun;
use crate::check::command::output::CheckFeedbackContext;
use crate::check::command::workflow::failure::CheckPublicOutputProgress;
use crate::check::command::workflow::failure::{
    combine_failure_effect_results, finish_check_error_report, CheckErrorReportFinish,
};
use crate::check::command::workflow::trailer::{
    check_report_passed, write_check_trailer, CompletedCheckRun,
};
use crate::check::command::{finish_check_report, CheckReportFinishContext};
use crate::check::{run_check_with_runner_and_caches, CheckRunSideEffects};
use crate::cli::{write_command_error_line, CommandError, ReportedCommandFailure};
use crate::logs::DiagnosticLogWriter;
use std::io::Write;
use std::time::Instant;

pub(super) fn run_prepared_check(context: PreparedCheckRun<'_, '_>) -> Result<(), CommandError> {
    let PreparedCheckRun {
        runtime,
        options,
        runner,
        check_caches,
        diagnostic_log,
        started,
        public_output_progress,
        progress_report,
        feedback_context,
        resolve_selected_diff_from_tree_oids,
    } = context;
    let runtime_root = runtime.root;
    let runtime_config = runtime.config;
    let runtime_expectation_identities = runtime.expectation_identities;
    let runtime_head_tree_oid = runtime.git_head_tree_oid().map(str::to_string);
    let shared_output = crate::check::command::output::SharedCheckOutput::stdout();
    let mut result_output = shared_output.clone();
    let records_result = run_check_with_runner_and_caches(
        runtime,
        options,
        runner,
        progress_report,
        CheckRunSideEffects {
            diagnostic_log: Some(&mut *diagnostic_log),
            result_output: Some(&mut result_output),
            live_report_output: Some(shared_output),
            caches: check_caches,
            resolve_selected_diff_from_tree_oids,
        },
    );
    let mut completed = match records_result {
        Ok(report) => CompletedCheckRun {
            report,
            error: None,
            failure_history_feedback: None,
        },
        Err(err) => CompletedCheckRun {
            report: *err.report,
            error: Some(err.error),
            failure_history_feedback: None,
        },
    };
    if let Some(head_tree_oid) = runtime_head_tree_oid.as_deref() {
        if completed
            .report
            .records
            .iter()
            .any(|record| !record.passed())
        {
            match check_caches.xpec_state.append_check_failure_history(
                runtime_root,
                head_tree_oid,
                runtime_config,
                runtime_expectation_identities,
                &completed.report,
            ) {
                Ok(feedback) => completed.failure_history_feedback = feedback,
                Err(error) => {
                    let history_error = format!("failed to update check failure history: {error}");
                    completed.error = Some(match completed.error.take() {
                        Some(primary) => format!("{primary}; also {history_error}"),
                        None => history_error,
                    });
                }
            }
        }
    }
    finish_completed_check(
        Some(diagnostic_log),
        &mut result_output,
        runner,
        &completed,
        started,
        public_output_progress,
        feedback_context,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_completed_check(
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    result_output: &mut dyn Write,
    runner: &mut crate::app::LazyAppServerRunner,
    completed: &CompletedCheckRun,
    started: Instant,
    public_output_progress: &mut CheckPublicOutputProgress,
    feedback_context: Option<CheckFeedbackContext>,
) -> Result<(), CommandError> {
    // [1h,2Z,KD,w] An engine-side failure can follow a publicly completed
    // expectation result. Report that command failure before the trailer, and
    // still attempt every remaining output even if its diagnostic write fails.
    let diagnostic_result = completed.error.as_ref().map_or(Ok(()), |error| {
        write_command_error_line(&CommandError::from(error.clone())).map_err(CommandError::from)
    });
    let completion_result = finish_completed_check_outputs(
        diagnostic_log,
        result_output,
        runner,
        completed,
        started,
        public_output_progress,
        feedback_context,
    );
    combine_failure_effect_results([diagnostic_result, completion_result])
}

fn finish_completed_check_outputs(
    mut diagnostic_log: Option<&mut DiagnosticLogWriter>,
    result_output: &mut dyn Write,
    runner: &mut crate::app::LazyAppServerRunner,
    completed: &CompletedCheckRun,
    started: Instant,
    public_output_progress: &mut CheckPublicOutputProgress,
    feedback_context: Option<CheckFeedbackContext>,
) -> Result<(), CommandError> {
    let trailer_result = write_check_trailer(
        runner,
        result_output,
        &completed.report,
        started,
        public_output_progress,
    );
    public_output_progress.mark_feedback_attempted();
    if let Err(err) = trailer_result {
        let Some(diagnostic_log) = diagnostic_log.as_deref_mut() else {
            return Err(CommandError::from(err));
        };
        // [2Z,KD,w] The trailer error is another command failure. The common
        // report finisher keeps repair/continue feedback for failed or pending
        // reports and emits command-error feedback instead of success/commit
        // guidance for an otherwise all-passed report.
        return finish_check_error_report(CheckErrorReportFinish {
            diagnostic_log,
            result_output,
            report: &completed.report,
            error: err,
            feedback_context,
            failure_history_feedback: completed.failure_history_feedback.as_ref(),
        });
    }
    let completed_error = completed.error.clone();
    finish_check_report(
        CheckReportFinishContext {
            diagnostic_log,
            result_output,
            // [ex] This is post-summary feedback, not a success-only message.
            // The `finally` contract emits it for interrupted default-source
            // runs too. Failed or pending reports keep their canonical wording;
            // an otherwise all-passed command failure gets the error action.
            feedback_context,
            failure_history_feedback: completed.failure_history_feedback.as_ref(),
        },
        &completed.report,
        completed_error.as_deref(),
    )?;
    if completed.error.is_none() && check_report_passed(&completed.report) {
        Ok(())
    } else {
        Err(CommandError::Reported(ReportedCommandFailure::Check))
    }
}
