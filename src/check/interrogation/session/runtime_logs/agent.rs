use crate::logs::{
    write_agent_failure_event, write_agent_missing_usage_event, write_agent_request_event,
    write_agent_response_event, AgentTurnLogRequest, DiagnosticLogResult, DiagnosticLogWriter,
};
use crate::token_usage::EvaluatorTurnUsage;

// These adapters preserve turn usage data for `logs::events`, which emits a
// separate `agent.token_usage` event with normalized counters for each turn.
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
    thread_id: &str,
    response: &str,
    turn_usage: &EvaluatorTurnUsage,
) -> DiagnosticLogResult<()> {
    write_agent_response_event(
        diagnostic_log,
        expectation_id,
        attempt,
        reason,
        thread_id,
        response,
        turn_usage,
    )
}

pub(crate) fn write_agent_turn_failure_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    thread_id: &str,
    error: &str,
    turn_usage: Option<&EvaluatorTurnUsage>,
) -> DiagnosticLogResult<()> {
    write_agent_failure_event(
        diagnostic_log,
        expectation_id,
        attempt,
        reason,
        thread_id,
        error,
        turn_usage,
    )
}

pub(crate) fn write_agent_turn_missing_usage_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    thread_id: &str,
    response: &str,
) -> DiagnosticLogResult<()> {
    write_agent_missing_usage_event(
        diagnostic_log,
        expectation_id,
        attempt,
        reason,
        thread_id,
        response,
    )
}
