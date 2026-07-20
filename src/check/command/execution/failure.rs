use crate::check::command::output::{
    continue_evaluation_message, render_check_agent_messages, write_stdout_record,
    write_summary_line,
};
use crate::check::command::print_token_usage_summary;
use crate::check::command::{finish_check_report, CheckReportFinishContext};
use crate::check::core::CheckRunReport;
use crate::check::interrogation::{
    write_check_lifecycle_finish_event, write_check_lifecycle_start_event,
};
use crate::cli::{write_command_error_line, CommandError};
use crate::logs::DiagnosticLogWriter;
use std::io::{self, Write};
use std::time::Instant;

pub(super) struct CheckErrorReportFinish<'b> {
    pub(super) diagnostic_log: &'b mut DiagnosticLogWriter,
    pub(super) result_output: &'b mut dyn Write,
    pub(super) report: &'b CheckRunReport,
    pub(super) error: String,
    pub(super) write_agent_message: bool,
    pub(super) need_to_commit: bool,
}

#[derive(Clone, Copy)]
pub(super) struct CheckFailureOutput {
    started: Instant,
    collection: CheckFailureCollection,
    default_feedback_eligible: bool,
    need_to_commit: bool,
}

#[derive(Clone, Copy)]
enum CheckFailureCollection {
    BeforeCollection,
    Collected {
        pending: usize,
        default_feedback_eligible: bool,
    },
}

impl CheckFailureOutput {
    pub(super) fn needs_pending_collection(self) -> bool {
        self.default_feedback_eligible
            && matches!(self.collection, CheckFailureCollection::BeforeCollection)
    }

    pub(super) fn with_pending(mut self, pending: usize) -> Self {
        self.collection = CheckFailureCollection::Collected {
            pending,
            default_feedback_eligible: self.default_feedback_eligible,
        };
        self
    }

    pub(super) fn has_collected_default_feedback_context(self) -> bool {
        matches!(
            self.collection,
            CheckFailureCollection::Collected {
                default_feedback_eligible: true,
                ..
            }
        )
    }

    pub(super) fn with_need_to_commit(mut self, need_to_commit: bool) -> Self {
        self.need_to_commit = need_to_commit;
        self
    }

    fn pending_count(self) -> usize {
        // [9b] Before config collection there are zero collected xpecs. After
        // collection, every xpec without a result is pending. Represent those
        // two canon states explicitly instead of fabricating result records.
        match self.collection {
            CheckFailureCollection::BeforeCollection => 0,
            CheckFailureCollection::Collected { pending, .. } => pending,
        }
    }

    fn feedback_messages(self) -> Vec<String> {
        match self.collection {
            // Collection did not finish, so evaluation is necessarily still
            // pending even though no complete xpec outcome domain exists for
            // the summary. Use the canonical pending feedback text directly;
            // do not invent an xpec count.
            CheckFailureCollection::BeforeCollection if self.default_feedback_eligible => {
                vec![continue_evaluation_message()]
            }
            CheckFailureCollection::Collected {
                pending,
                default_feedback_eligible: true,
            } => render_check_agent_messages(&[], pending, self.need_to_commit),
            CheckFailureCollection::BeforeCollection
            | CheckFailureCollection::Collected {
                default_feedback_eligible: false,
                ..
            } => Vec::new(),
        }
    }
}

pub(super) fn finish_check_error_report(
    context: CheckErrorReportFinish<'_>,
) -> Result<(), CommandError> {
    let error = context.error;
    // [7N] This path follows a failed `write_check_trailer`, which already
    // attempted both unconditional trailer parts independently. Finish the
    // remaining feedback and lifecycle-log parts without duplicating either
    // trailer line.
    finish_check_report(
        CheckReportFinishContext {
            diagnostic_log: Some(context.diagnostic_log),
            result_output: context.result_output,
            write_agent_message: context.write_agent_message,
            need_to_commit: context.need_to_commit,
        },
        context.report,
        Some(&error),
    )?;
    Err(error.into())
}

pub(super) fn fail_check_before_selection(
    diagnostic_log: &mut DiagnosticLogWriter,
    trailer_attempted: &mut bool,
    output: CheckFailureOutput,
    err: String,
) -> Result<(), CommandError> {
    // [7N] Failed runtime-log storage must not suppress the public `finally`
    // output. Preserve the start-log failure in the command error, then
    // continue through the common trailer and finish-log path.
    let err = match write_check_lifecycle_start_event(diagnostic_log, None, Vec::new()) {
        Ok(()) => err,
        Err(start_error) => {
            format!("{err}; also failed to write check lifecycle start event: {start_error}")
        }
    };
    finish_check_failure(diagnostic_log, trailer_attempted, output, err)
}

pub(super) fn start_check_or_fail(
    diagnostic_log: &mut DiagnosticLogWriter,
    selected_ids: Vec<String>,
    trailer_attempted: &mut bool,
    output: CheckFailureOutput,
) -> Result<(), CommandError> {
    // [7N] Selection establishes the lifecycle boundary. If start-event storage
    // itself fails, the command still traverses the same public trailer and
    // best-effort finish-event path as every later failure.
    match write_check_lifecycle_start_event(diagnostic_log, None, selected_ids) {
        Ok(()) => Ok(()),
        Err(err) => fail_check_after_selection(
            diagnostic_log,
            trailer_attempted,
            output,
            format!("failed to write check lifecycle start event: {err}"),
        ),
    }
}

pub(super) fn fail_check_after_selection(
    diagnostic_log: &mut DiagnosticLogWriter,
    trailer_attempted: &mut bool,
    output: CheckFailureOutput,
    err: String,
) -> Result<(), CommandError> {
    // [7N] Once selection has completed, retain its pending set for canonical
    // feedback. Earlier failures reach the same finisher after their default
    // config and tree feedback context has been prepared.
    finish_check_failure(diagnostic_log, trailer_attempted, output, err)
}

fn finish_check_failure(
    diagnostic_log: &mut DiagnosticLogWriter,
    trailer_attempted: &mut bool,
    output: CheckFailureOutput,
    err: String,
) -> Result<(), CommandError> {
    // [1h,Y8] Once a check-specific failure is known, its diagnostic is
    // eligible immediately. Flush it before unrelated `finally` effects, then
    // return a sentinel so the outer command boundary does not print it twice.
    let diagnostic_result =
        write_command_error_line(&CommandError::from(err.clone())).map_err(CommandError::from);
    // [7N,9b,hJ] This check-only `finally` path always emits token usage and a
    // summary. Once collection succeeds, every collected xpec without a result
    // is pending. Eligible default-source failures then emit normal
    // count-derived feedback; failures before collection cannot.
    *trailer_attempted = true;
    let trailer_result = write_required_check_failure_outputs(output);
    // [7N] The public trailer and lifecycle logging are independent failure
    // effects. Attempt the log even when public output fails, and never let a
    // logging failure suppress the unconditional check trailer.
    let finish_result =
        write_check_error_finish_event(diagnostic_log, &err).map_err(CommandError::from);
    combine_failure_effect_results([diagnostic_result, trailer_result, finish_result])?;
    Err(CommandError::CheckFailed)
}

fn combine_failure_effect_results<const N: usize>(
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

pub(super) fn started_check_output(
    started: Instant,
    default_feedback_eligible: bool,
) -> CheckFailureOutput {
    requested_check_output(started, default_feedback_eligible)
}

pub(super) fn requested_check_output(
    started: Instant,
    default_feedback_eligible: bool,
) -> CheckFailureOutput {
    CheckFailureOutput {
        started,
        collection: CheckFailureCollection::BeforeCollection,
        default_feedback_eligible,
        need_to_commit: false,
    }
}

pub(super) fn collected_check_output(
    started: Instant,
    pending: usize,
    default_feedback_eligible: bool,
) -> CheckFailureOutput {
    CheckFailureOutput {
        started,
        collection: CheckFailureCollection::Collected {
            pending,
            default_feedback_eligible,
        },
        default_feedback_eligible,
        need_to_commit: false,
    }
}

pub(super) fn write_required_check_failure_outputs(
    output: CheckFailureOutput,
) -> Result<(), CommandError> {
    // [Y8,7N] These three `finally` effects are independent of each other and
    // of the command-specific diagnostic already flushed at the failure
    // boundary. Compute every result before combining errors so a failure in
    // any output channel cannot suppress token usage, summary, or feedback.
    let pending = output.pending_count();
    let report = CheckRunReport {
        records: Vec::new(),
        cached: Vec::new(),
        pending,
    };
    let token_usage_result = print_token_usage_summary(None).map_err(CommandError::from);
    let summary_result = write_summary_line(&mut io::stdout(), &report, output.started.elapsed())
        .map_err(CommandError::from);
    let feedback_result = write_check_failure_feedback(output);
    combine_failure_effect_results([token_usage_result, summary_result, feedback_result])
}

fn write_check_failure_feedback(output: CheckFailureOutput) -> Result<(), CommandError> {
    // [7N] Feedback is an independent `finally` effect for a selected
    // default-source run. An incomplete collection uses the canonical pending
    // action without inventing xpec counts; completed collection uses the
    // normal count-derived branches. Attempt feedback after both trailer
    // writes even if either output channel has failed.
    for message in output.feedback_messages() {
        let line = format!("{message}\n");
        write_stdout_record(
            &mut io::stdout(),
            line.as_bytes(),
            "pre-report check agent message",
        )?;
    }
    Ok(())
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

    #[test] // xpec: 7N,2
    fn incomplete_collection_uses_canonical_pending_feedback() {
        let before_collection = requested_check_output(Instant::now(), true);
        let collected = before_collection.with_pending(1);

        assert_eq!(
            before_collection.feedback_messages(),
            vec![continue_evaluation_message()]
        );
        assert_eq!(
            collected.feedback_messages(),
            vec![continue_evaluation_message()]
        );
    }
}
