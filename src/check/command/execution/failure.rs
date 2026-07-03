use crate::check::command::output::write_summary_line;
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

pub(super) enum CheckFailureOutput {
    StartedCheck { started: Instant },
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
    output: CheckFailureOutput,
    err: String,
) -> Result<(), CommandError> {
    write_check_lifecycle_start_event(diagnostic_log, start_query, Vec::new())?;
    fail_check_after_start(diagnostic_log, finish_query, output, err)
}

pub(super) fn fail_check_after_start(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    output: CheckFailureOutput,
    err: String,
) -> Result<(), CommandError> {
    write_check_error_finish_event(diagnostic_log, query, &err).map_err(CommandError::from)?;
    if !query {
        let CheckFailureOutput::StartedCheck { started } = output;
        // Once a `canon check` run has started, it still owns the check trailer
        // surface even when it fails before an expectation report exists.
        print_token_usage_summary(None).map_err(CommandError::from)?;
        write_empty_check_summary(started)?;
    }
    Err(err.into())
}

pub(super) fn started_check_output(started: Instant) -> CheckFailureOutput {
    CheckFailureOutput::StartedCheck { started }
}

fn write_empty_check_summary(started: Instant) -> Result<(), CommandError> {
    let report = CheckRunReport {
        records: Vec::new(),
        cached: Vec::new(),
        blocked_hooks: Vec::new(),
        skipped: 0,
    };
    write_summary_line(&mut io::stdout(), &report, started.elapsed()).map_err(CommandError::from)
}

pub(super) fn write_check_error_finish_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    err: &str,
) -> Result<(), String> {
    write_check_lifecycle_finish_event(diagnostic_log, query, Some(err))
        .map_err(|err| err.to_string())
}
