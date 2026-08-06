use crate::logs::{
    write_check_finish_event, write_check_start_event, write_query_finish_event,
    write_query_start_event, DiagnosticLogResult, DiagnosticLogWriter,
};

// Command execution owns lifecycle bracketing because it covers selection,
// preparation, output, and early failures. These adapters keep those events at
// the interrogation boundary without mixing them with agent-turn logging.
pub(crate) fn write_check_lifecycle_start_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: Option<bool>,
    candidates: Vec<String>,
) -> DiagnosticLogResult<()> {
    write_check_start_event(diagnostic_log, query, candidates)
}

pub(crate) fn write_check_lifecycle_finish_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    error: Option<&str>,
) -> DiagnosticLogResult<()> {
    write_check_finish_event(diagnostic_log, query, error)
}

pub(crate) fn write_query_lifecycle_start_event(
    diagnostic_log: &mut DiagnosticLogWriter,
) -> DiagnosticLogResult<()> {
    write_query_start_event(diagnostic_log)
}

pub(crate) fn write_query_lifecycle_finish_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    error: Option<&str>,
) -> DiagnosticLogResult<()> {
    write_query_finish_event(diagnostic_log, error)
}
