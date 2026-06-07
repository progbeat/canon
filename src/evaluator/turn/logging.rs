use super::types::RawTurnResponse;
use super::{EvaluatorFailureKind, EvaluatorTurnContext, ThreadLifecycleLog};
use crate::check::{
    write_agent_turn_failure_event, write_agent_turn_missing_usage_event,
    write_agent_turn_request_event, write_agent_turn_response_event,
};
use crate::evaluator::{EvaluatorError, EvaluatorRunner, EVALUATOR_BASE_INSTRUCTIONS};
use crate::logs::{
    AgentTurnLogRequest, DiagnosticLogResult, DiagnosticLogWriter, ThreadLifecycleEventFields,
    ThreadRestartEventFields,
};

pub(super) fn ask_and_log<R: EvaluatorRunner>(
    runner: &mut R,
    turn: &EvaluatorTurnContext<'_>,
    prompt: &str,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
) -> Result<RawTurnResponse, EvaluatorError> {
    write_optional_diagnostic_log(diagnostic_log, |writer| {
        write_agent_turn_request_event(
            writer,
            expectation_id,
            attempt,
            reason,
            AgentTurnLogRequest {
                session_id: turn.session_id,
                prompt,
                model: turn.model,
                thinking: turn.thinking,
            },
        )
    });
    let response = match runner.ask(turn.session_id, prompt, turn.model, turn.thinking) {
        Ok(response) => response,
        Err(err) => {
            let turn_usage = runner.take_last_turn_usage();
            write_optional_diagnostic_log(diagnostic_log, |writer| {
                write_agent_turn_failure_event(
                    writer,
                    expectation_id,
                    attempt,
                    reason,
                    turn.session_id,
                    err.message_str(),
                    turn_usage.as_ref(),
                )
            });
            return Err(err);
        }
    };
    let turn_usage = runner.take_last_turn_usage();
    let response_usage = turn_usage.as_ref().map(|turn_usage| turn_usage.usage);
    let missing_turn_usage = turn_usage.is_none();
    if missing_turn_usage {
        // A response without usage violates the app-server turn contract, so
        // it is not logged as a completed `agent.response`.
        write_optional_diagnostic_log(diagnostic_log, |writer| {
            write_agent_turn_missing_usage_event(
                writer,
                expectation_id,
                attempt,
                reason,
                turn.session_id,
                &response,
            )
        });
    } else {
        let turn_usage = turn_usage
            .as_ref()
            .expect("missing_turn_usage is false when usage exists");
        write_optional_diagnostic_log(diagnostic_log, |writer| {
            write_agent_turn_response_event(
                writer,
                expectation_id,
                attempt,
                reason,
                turn.session_id,
                &response,
                turn_usage,
            )
        });
    }
    if missing_turn_usage {
        return Err(EvaluatorError::failure(
            EvaluatorFailureKind::UnknownAppServer,
            "missing evaluator turn usage",
        ));
    }
    let context_compacted = turn_usage
        .as_ref()
        .is_some_and(|turn_usage| !turn_usage.context_compaction_events.is_empty());
    Ok(RawTurnResponse {
        text: response,
        usage: response_usage,
        context_compacted,
    })
}

fn write_optional_diagnostic_log(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    write: impl FnOnce(&mut DiagnosticLogWriter) -> DiagnosticLogResult<()>,
) {
    // Diagnostic logs are observability data; write failures must not replace
    // the evaluator result, evaluator error, or app-server contract error.
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        let _ = write(writer);
    }
}

pub(crate) fn write_thread_lifecycle_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    lifecycle_log: &ThreadLifecycleLog,
    enforced_scope: &[String],
    model: Option<&str>,
    thinking: &str,
) {
    write_optional_diagnostic_log(diagnostic_log, |writer| {
        crate::logs::write_thread_lifecycle_event(
            writer,
            &ThreadLifecycleEventFields {
                event: lifecycle_log.event,
                session_id: &lifecycle_log.session_id,
                scope: enforced_scope,
                model,
                thinking,
                base_instructions: EVALUATOR_BASE_INSTRUCTIONS,
                developer_instructions: &lifecycle_log.developer_instructions,
            },
        )
    });
}

pub(crate) fn write_thread_restart_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    session_id: &str,
    expectation_id: Option<&str>,
    enforced_scope: &[String],
    model: Option<&str>,
    developer_instructions: &str,
    reason: &str,
) {
    write_optional_diagnostic_log(diagnostic_log, |writer| {
        crate::logs::write_thread_restart_event(
            writer,
            &ThreadRestartEventFields {
                session_id,
                expectation_id,
                scope: enforced_scope,
                model,
                base_instructions: EVALUATOR_BASE_INSTRUCTIONS,
                developer_instructions,
                reason,
            },
        )
    });
}
