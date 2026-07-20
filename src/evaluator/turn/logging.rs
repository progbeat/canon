use super::types::RawTurnResponse;
use super::{EvaluatorFailureKind, EvaluatorTurnContext, ThreadLifecycleLog};
use crate::check::{
    write_agent_turn_failure_event, write_agent_turn_missing_usage_event,
    write_agent_turn_request_event, write_agent_turn_response_event,
};
use crate::evaluator::{EvaluatorDynamicToolHandler, EvaluatorError, EvaluatorRunner};
use crate::logs::{
    AgentTurnLogRequest, DiagnosticLogResult, DiagnosticLogWriter, ThreadLifecycleEventFields,
    ThreadRestartEventFields,
};
use serde_json::Value;

pub(super) struct LoggedTurnRequest<'a> {
    pub(super) turn: &'a EvaluatorTurnContext<'a>,
    pub(super) prompt: &'a str,
    pub(super) expectation_id: Option<&'a str>,
    pub(super) attempt: usize,
    pub(super) reason: &'a str,
    pub(super) output_schema: &'a Value,
}

pub(super) fn ask_and_log<R: EvaluatorRunner>(
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    request: LoggedTurnRequest<'_>,
    dynamic_tool_handler: Option<&mut dyn EvaluatorDynamicToolHandler>,
) -> Result<RawTurnResponse, EvaluatorError> {
    write_optional_diagnostic_log(diagnostic_log, |writer| {
        write_agent_turn_request_event(
            writer,
            request.expectation_id,
            request.attempt,
            request.reason,
            AgentTurnLogRequest {
                session_id: request.turn.session_id,
                prompt: request.prompt,
                model: request.turn.model,
                thinking: request.turn.thinking,
            },
        )
    })?;
    let response = match runner.ask(
        request.turn.session_id,
        request.prompt,
        request.turn.model,
        request.turn.thinking,
        request.output_schema,
        dynamic_tool_handler,
    ) {
        Ok(response) => response,
        Err(err) => {
            let turn_usage = runner.take_last_turn_usage();
            if let Err(log_error) = write_optional_diagnostic_log(diagnostic_log, |writer| {
                write_agent_turn_failure_event(
                    writer,
                    request.expectation_id,
                    request.attempt,
                    request.reason,
                    request.turn.session_id,
                    err.message_str(),
                    turn_usage.as_ref(),
                )
            }) {
                return Err(EvaluatorError::message(format!(
                    "{}; failed to write evaluator failure runtime log: {}",
                    err.message_str(),
                    log_error
                )));
            }
            return Err(err);
        }
    };
    let turn_usage = runner.take_last_turn_usage();
    let missing_turn_usage = turn_usage.is_none();
    if missing_turn_usage {
        // A response without usage violates the app-server turn contract, so
        // it is not logged as a completed `agent.response`.
        write_optional_diagnostic_log(diagnostic_log, |writer| {
            write_agent_turn_missing_usage_event(
                writer,
                request.expectation_id,
                request.attempt,
                request.reason,
                request.turn.session_id,
                &response,
            )
        })?;
    } else {
        let turn_usage = turn_usage
            .as_ref()
            .expect("missing_turn_usage is false when usage exists");
        write_optional_diagnostic_log(diagnostic_log, |writer| {
            write_agent_turn_response_event(
                writer,
                request.expectation_id,
                request.attempt,
                request.reason,
                request.turn.session_id,
                &response,
                turn_usage,
            )
        })?;
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
        context_compacted,
    })
}

fn write_optional_diagnostic_log(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    write: impl FnOnce(&mut DiagnosticLogWriter) -> DiagnosticLogResult<()>,
) -> Result<(), EvaluatorError> {
    // [7N] A configured runtime log is part of the check observability
    // contract. Its write failures must become command errors rather than
    // silently omitting evaluator communication. When an evaluator failure
    // already exists, `ask_and_log` reports both errors above.
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        write(writer)?;
    }
    Ok(())
}

pub(crate) fn write_thread_lifecycle_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    lifecycle_log: &ThreadLifecycleLog,
    expectation_id: Option<&str>,
    enforced_scope: &[String],
    model: Option<&str>,
    thinking: &str,
) -> Result<(), EvaluatorError> {
    write_optional_diagnostic_log(diagnostic_log, |writer| {
        crate::logs::write_thread_lifecycle_event(
            writer,
            &ThreadLifecycleEventFields {
                event: lifecycle_log.event,
                session_id: &lifecycle_log.session_id,
                expectation_id,
                scope: enforced_scope,
                model,
                thinking,
                base_instructions: &lifecycle_log.base_instructions,
                developer_instructions: &lifecycle_log.developer_instructions,
                reuse_context: &lifecycle_log.reuse_context,
            },
        )
    })
}

pub(crate) fn write_thread_restart_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    lifecycle_log: &ThreadLifecycleLog,
    expectation_id: Option<&str>,
    enforced_scope: &[String],
    model: Option<&str>,
    reason: &str,
) -> Result<(), EvaluatorError> {
    write_optional_diagnostic_log(diagnostic_log, |writer| {
        crate::logs::write_thread_restart_event(
            writer,
            &ThreadRestartEventFields {
                session_id: &lifecycle_log.session_id,
                expectation_id,
                scope: enforced_scope,
                model,
                base_instructions: &lifecycle_log.base_instructions,
                developer_instructions: &lifecycle_log.developer_instructions,
                reason,
            },
        )
    })
}
