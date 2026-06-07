use crate::logs::{
    write_agent_failure_event, write_agent_missing_usage_event, write_agent_request_event,
    write_agent_response_event, write_check_finish_event, write_check_start_event,
    write_query_finish_event, write_query_start_event, AgentTurnLogRequest, DiagnosticLogResult,
    DiagnosticLogWriter,
};
use crate::token_usage_types::EvaluatorTurnUsage;

// Command execution owns lifecycle bracketing because it must cover selection,
// preparation, output, and early failures. The concrete call sites are
// `check::command::execution::{run,failure,query}` and
// `check::command::completion`; this module keeps those events routed through
// the interrogation logging boundary alongside evaluator communication events.
pub(crate) fn write_check_lifecycle_start_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: Option<bool>,
    selected: Vec<String>,
) -> DiagnosticLogResult<()> {
    write_check_start_event(diagnostic_log, query, selected)
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

pub(crate) fn write_agent_turn_request_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    request: AgentTurnLogRequest<'_>,
) -> DiagnosticLogResult<()> {
    write_agent_request_event(diagnostic_log, expectation_id, attempt, reason, request)
}

pub(crate) fn write_agent_turn_response_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    session_id: &str,
    response: &str,
    turn_usage: &EvaluatorTurnUsage,
) -> DiagnosticLogResult<()> {
    write_agent_response_event(
        diagnostic_log,
        expectation_id,
        attempt,
        reason,
        session_id,
        response,
        turn_usage,
    )
}

pub(crate) fn write_agent_turn_failure_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    session_id: &str,
    error: &str,
    turn_usage: Option<&EvaluatorTurnUsage>,
) -> DiagnosticLogResult<()> {
    write_agent_failure_event(
        diagnostic_log,
        expectation_id,
        attempt,
        reason,
        session_id,
        error,
        turn_usage,
    )
}

pub(crate) fn write_agent_turn_missing_usage_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    session_id: &str,
    response: &str,
) -> DiagnosticLogResult<()> {
    write_agent_missing_usage_event(
        diagnostic_log,
        expectation_id,
        attempt,
        reason,
        session_id,
        response,
    )
}
