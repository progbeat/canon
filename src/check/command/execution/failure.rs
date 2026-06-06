use crate::check::command::finish::{finish_check_report, CheckReportFinishContext};
use crate::check::command::reporting::write_check_finish_event;
use crate::check::core::types::CheckRunReport;
use crate::check::run::lazy_reset::apply_lazy_full_scope_reset_for_cached;
use crate::check::CheckRunCaches;
use crate::cli::CommandError;
use crate::config_types::CheckConfig;
use crate::logs::DiagnosticLogWriter;
use serde_json::json;
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

pub(super) fn write_check_start_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: Option<bool>,
    selected: Vec<String>,
) -> Result<(), CommandError> {
    let mut fields = Vec::new();
    if let Some(query) = query {
        fields.push(("query", json!(query)));
    }
    fields.push(("selected", json!(selected)));
    diagnostic_log
        .write_event("info", "check.start", &fields)
        .map_err(CommandError::from)
}

pub(super) fn fail_check_before_selection(
    root: &Path,
    diagnostic_log: &mut DiagnosticLogWriter,
    start_query: Option<bool>,
    finish_query: bool,
    errors: usize,
    err: String,
) -> Result<(), CommandError> {
    write_check_start_event(diagnostic_log, start_query, Vec::new())?;
    fail_check_after_start(root, diagnostic_log, finish_query, errors, err)
}

pub(super) fn fail_check_after_start(
    root: &Path,
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    errors: usize,
    err: String,
) -> Result<(), CommandError> {
    write_check_error_finish_event(root, diagnostic_log, query, errors, &err)
        .map_err(CommandError::from)?;
    Err(err.into())
}

pub(super) fn write_check_error_finish_event(
    root: &Path,
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    _errors: usize,
    err: &str,
) -> Result<(), String> {
    let mut finish_error = err.to_string();
    if let Err(reset_err) = apply_lazy_full_scope_reset_for_cached(root, 0, &[], diagnostic_log) {
        finish_error.push_str("; lazy full-scope reset failed: ");
        finish_error.push_str(&reset_err);
    }
    write_check_finish_event(diagnostic_log, query, Some(&finish_error))
}
