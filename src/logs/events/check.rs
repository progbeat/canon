use crate::logs::error::DiagnosticLogResult;
use crate::logs::DiagnosticLogWriter;
use serde_json::json;

// Check lifecycle event helpers are routed through
// `check::interrogation::session` so lifecycle logging is visible beside
// evaluator communication logging while the JSON event schemas stay centralized.
pub(crate) fn write_check_start_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: Option<bool>,
    candidates: Vec<String>,
) -> DiagnosticLogResult<()> {
    let mut fields = Vec::new();
    if let Some(query) = query {
        fields.push(("query", json!(query)));
    }
    // [gN] A lifecycle start precedes cache filtering. These are candidates;
    // the canonical Selected set is established only before evaluation.
    fields.push(("candidates", json!(candidates)));
    diagnostic_log.emit_event("info", "check.start", &fields)
}

pub(crate) fn write_query_start_event(
    diagnostic_log: &mut DiagnosticLogWriter,
) -> DiagnosticLogResult<()> {
    write_check_start_event(diagnostic_log, Some(true), Vec::new())
}

pub(crate) fn write_query_finish_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    err: Option<&str>,
) -> DiagnosticLogResult<()> {
    write_check_finish_event(diagnostic_log, true, err)
}

pub(crate) fn write_check_finish_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    error: Option<&str>,
) -> DiagnosticLogResult<()> {
    let mut fields = vec![("query", json!(query))];
    if let Some(error) = error {
        fields.push(("error", json!(error)));
    }
    diagnostic_log.emit_event("info", "check.finish", &fields)
}
