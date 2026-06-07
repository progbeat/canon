use crate::check::command::finish::{finish_check_report, CheckReportFinishContext};
use crate::check::core::types::CheckRunReport;
use crate::check::CheckRunCaches;
use crate::cli::CommandError;
use crate::config_types::CheckConfig;
use crate::logs::{write_check_finish_event, write_check_start_event, DiagnosticLogWriter};
use std::io::Write;
use std::path::Path;

pub(super) struct CheckErrorReportFinish<'a, 'b> {
    pub(super) root: &'a Path,
    pub(super) config: &'a CheckConfig,
    pub(super) diagnostic_log: &'b mut DiagnosticLogWriter,
    pub(super) result_output: &'b mut dyn Write,
    pub(super) check_caches: &'b mut CheckRunCaches,
    pub(super) report: &'b CheckRunReport,
    pub(super) error: String,
}

pub(super) fn finish_check_error_report(
    context: CheckErrorReportFinish<'_, '_>,
) -> Result<(), CommandError> {
    let error = context.error;
    finish_check_report(
        CheckReportFinishContext {
            root: context.root,
            config: context.config,
            diagnostic_log: context.diagnostic_log,
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
    err: String,
) -> Result<(), CommandError> {
    write_check_start_event(diagnostic_log, start_query, Vec::new())?;
    fail_check_after_start(diagnostic_log, finish_query, err)
}

pub(super) fn fail_check_after_start(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    err: String,
) -> Result<(), CommandError> {
    write_check_error_finish_event(diagnostic_log, query, &err).map_err(CommandError::from)?;
    Err(err.into())
}

pub(super) fn write_check_error_finish_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    err: &str,
) -> Result<(), String> {
    write_check_finish_event(diagnostic_log, query, Some(err)).map_err(|err| err.to_string())
}
