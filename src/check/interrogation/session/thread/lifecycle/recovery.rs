use super::super::super::InterrogationSession;
use super::super::{ThreadTurnContext, ThreadTurnRequest};
use super::selection::PreparedThread;
use super::startup::{start_new_thread_after_rendering, ThreadStartupContext};
use super::turn::log_thread_lifecycle_and_ask;
use crate::evaluator::{
    write_thread_restart_event, EvaluatorAttemptReason, EvaluatorError, EvaluatorFailureKind,
    EvaluatorRunner, ParsedTurnResponse, ThreadLifecycleLog,
};

pub(super) fn restart_thread_and_ask<R: EvaluatorRunner>(
    context: &mut ThreadTurnContext<'_, '_, '_, R>,
    prepared: &PreparedThread,
    request: &ThreadTurnRequest<'_>,
    previous_lifecycle_log: &ThreadLifecycleLog,
    restart_reason: &str,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    write_thread_restart_event(
        context.diagnostic_log,
        previous_lifecycle_log,
        request.expectation_id,
        request.enforced_scope,
        request.model,
        restart_reason,
    )?;
    // [qv] A retry classified as fresh always sends a new thread/start. It
    // must not select any other reusable thread with equivalent instructions.
    let selection = start_new_thread_after_rendering(
        context.runtime,
        context.runner,
        request,
        ThreadStartupContext {
            workspace: &prepared.workspace,
            dynamic_tools: &prepared.dynamic_tools,
            evaluator_config_identity: &prepared.evaluator_config_identity,
            prerender_reuse_key: &prepared.prerender_reuse_key,
            prepared_instructions: &prepared.prepared_instructions,
            interrogation_session: context.interrogation_session,
            evaluation_context: previous_lifecycle_log.evaluation_context.clone(),
        },
    )?;
    let mut retry_request = request.clone();
    retry_request.attempt = context
        .attempt_sequence
        .next(EvaluatorAttemptReason::ThreadRestart);
    let response = log_thread_lifecycle_and_ask(context, &selection.lifecycle_log, &retry_request)
        .or_else(|err| {
            fail_after_thread_error(
                context.interrogation_session,
                Some(&selection.lifecycle_log.thread_id),
                err,
            )
        })?;
    Ok(response)
}

pub(super) fn fail_after_thread_error<T>(
    interrogation_session: &mut InterrogationSession,
    thread_id: Option<&str>,
    err: EvaluatorError,
) -> Result<T, EvaluatorError> {
    if err
        .kind()
        .is_some_and(EvaluatorFailureKind::invalidates_thread)
    {
        // Reuse applies only to successful, still-live evaluator threads. An
        // evaluator failure invalidates its own conversational context, not
        // unrelated threads in the run-level registry. A failed thread start
        // has no registered context to discard.
        if let Some(thread_id) = thread_id {
            interrogation_session.discard_thread(thread_id);
        }
    }
    Err(err)
}
