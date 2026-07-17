use crate::check::command::output::{
    render_check_agent_messages, write_stdout_record, write_summary_line,
};
use crate::check::command::print_token_usage_summary;
use crate::check::command::{finish_check_report, CheckReportFinishContext};
use crate::check::core::CheckRunReport;
use crate::check::interrogation::{
    write_check_lifecycle_finish_event, write_check_lifecycle_start_event,
};
use crate::check::CheckRunCaches;
use crate::cli::CommandError;
use crate::logs::DiagnosticLogWriter;
use std::io::{self, Write};
use std::time::Instant;

pub(super) struct CheckErrorReportFinish<'b> {
    pub(super) diagnostic_log: &'b mut DiagnosticLogWriter,
    pub(super) result_output: &'b mut dyn Write,
    pub(super) check_caches: &'b mut CheckRunCaches,
    pub(super) report: &'b CheckRunReport,
    pub(super) error: String,
    pub(super) write_token_usage: bool,
}

#[derive(Clone, Copy)]
pub(super) struct CheckFailureOutput {
    started: Instant,
    pending: Option<usize>,
    write_agent_message: bool,
}

impl CheckFailureOutput {
    pub(super) fn needs_pending_collection(self) -> bool {
        self.write_agent_message && self.pending.is_none()
    }

    pub(super) fn with_pending(mut self, pending: usize) -> Self {
        self.pending = Some(pending);
        self
    }
}

pub(super) fn finish_check_error_report(
    context: CheckErrorReportFinish<'_>,
) -> Result<(), CommandError> {
    let error = context.error;
    if context.write_token_usage {
        print_token_usage_summary(None).map_err(CommandError::from)?;
    }
    finish_check_report(
        CheckReportFinishContext {
            diagnostic_log: Some(context.diagnostic_log),
            result_output: context.result_output,
            check_caches: context.check_caches,
            write_agent_message: false,
        },
        context.report,
        Some(&error),
    )?;
    Err(error.into())
}

pub(super) fn fail_check_before_selection(
    diagnostic_log: &mut DiagnosticLogWriter,
    start_query: Option<bool>,
    finish_query: bool,
    trailer_attempted: &mut bool,
    output: CheckFailureOutput,
    err: String,
) -> Result<(), CommandError> {
    write_check_lifecycle_start_event(diagnostic_log, start_query, Vec::new())?;
    fail_check_after_start(diagnostic_log, finish_query, trailer_attempted, output, err)
}

pub(super) fn fail_check_after_start(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    trailer_attempted: &mut bool,
    output: CheckFailureOutput,
    err: String,
) -> Result<(), CommandError> {
    write_check_error_finish_event(diagnostic_log, query, &err).map_err(CommandError::from)?;
    if !query {
        // [v1] The `finally` path always emits token usage and a summary.
        // Once collection succeeds, every collected xpec without a result is
        // pending, so default-source feedback can use the normal count-derived
        // continuation branch. Before collection, the empty report still
        // follows the canon's normal feedback function.
        *trailer_attempted = true;
        write_check_failure_trailer(output)?;
    }
    Err(err.into())
}

pub(super) fn started_check_output(
    started: Instant,
    write_agent_message: bool,
) -> CheckFailureOutput {
    requested_check_output(started, write_agent_message)
}

pub(super) fn requested_check_output(
    started: Instant,
    write_agent_message: bool,
) -> CheckFailureOutput {
    CheckFailureOutput {
        started,
        pending: None,
        write_agent_message,
    }
}

pub(super) fn collected_check_output(
    started: Instant,
    pending: usize,
    write_agent_message: bool,
) -> CheckFailureOutput {
    CheckFailureOutput {
        started,
        pending: Some(pending),
        write_agent_message,
    }
}

pub(super) fn write_check_failure_trailer(output: CheckFailureOutput) -> Result<(), CommandError> {
    print_token_usage_summary(None).map_err(CommandError::from)?;
    let report = CheckRunReport {
        records: Vec::new(),
        cached: Vec::new(),
        skipped: output.pending.unwrap_or(0),
    };
    write_summary_line(&mut io::stdout(), &report, output.started.elapsed())
        .map_err(CommandError::from)?;
    if output.write_agent_message {
        for message in render_check_agent_messages(&[], 0, 0, report.skipped) {
            let line = format!("{message}\n");
            write_stdout_record(
                &mut io::stdout(),
                line.as_bytes(),
                "pre-report check agent message",
            )?;
        }
    }
    Ok(())
}

pub(super) fn write_check_error_finish_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    err: &str,
) -> Result<(), String> {
    write_check_lifecycle_finish_event(diagnostic_log, query, Some(err))
        .map_err(|err| err.to_string())
}
