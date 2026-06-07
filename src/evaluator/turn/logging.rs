use super::types::RawTurnResponse;
use super::{EvaluatorFailureKind, EvaluatorTurnContext, ThreadLifecycleLog};
use crate::check::{
    write_agent_turn_failure_event, write_agent_turn_missing_usage_event,
    write_agent_turn_request_event, write_agent_turn_response_event,
};
use crate::evaluator::prompt::EVALUATOR_BASE_INSTRUCTIONS;
use crate::evaluator::types::{EvaluatorError, EvaluatorRunner};
use crate::logs::{
    AgentTurnLogRequest, DiagnosticLogWriter, ThreadLifecycleEventFields, ThreadRestartEventFields,
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
    if let Some(writer) = diagnostic_log.as_deref_mut() {
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
        )?;
    }
    let response = match runner.ask(turn.session_id, prompt, turn.model, turn.thinking) {
        Ok(response) => response,
        Err(err) => {
            let turn_usage = runner.take_last_turn_usage();
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                write_agent_turn_failure_event(
                    writer,
                    expectation_id,
                    attempt,
                    reason,
                    turn.session_id,
                    err.message_str(),
                    turn_usage.as_ref(),
                )?;
            }
            return Err(err);
        }
    };
    let turn_usage = runner.take_last_turn_usage();
    let response_usage = turn_usage.as_ref().map(|turn_usage| turn_usage.usage);
    let missing_turn_usage = diagnostic_log.is_some() && turn_usage.is_none();
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        if missing_turn_usage {
            // A response without usage violates the app-server turn contract,
            // so it is not logged as a completed `agent.response`.
            write_agent_turn_missing_usage_event(
                writer,
                expectation_id,
                attempt,
                reason,
                turn.session_id,
                &response,
            )?;
        } else {
            let turn_usage = turn_usage
                .as_ref()
                .expect("missing_turn_usage is false when usage exists");
            write_agent_turn_response_event(
                writer,
                expectation_id,
                attempt,
                reason,
                turn.session_id,
                &response,
                turn_usage,
            )?;
        }
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

pub(crate) fn write_thread_lifecycle_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    lifecycle_log: &ThreadLifecycleLog,
    enforced_scope: &[String],
    model: Option<&str>,
    thinking: &str,
) -> Result<(), String> {
    let Some(writer) = diagnostic_log.as_deref_mut() else {
        return Ok(());
    };
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
    .map_err(|err| err.to_string())
}

pub(crate) fn write_thread_restart_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    session_id: &str,
    expectation_id: Option<&str>,
    enforced_scope: &[String],
    model: Option<&str>,
    developer_instructions: &str,
    reason: &str,
) -> Result<(), String> {
    let Some(writer) = diagnostic_log.as_deref_mut() else {
        return Ok(());
    };
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
    .map_err(|err| err.to_string())
}
