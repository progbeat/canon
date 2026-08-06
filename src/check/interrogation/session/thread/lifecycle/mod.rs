mod recovery;
mod selection;
mod startup;
mod turn;

use super::{ThreadTurnContext, ThreadTurnRequest};
use crate::evaluator::{EvaluatorError, EvaluatorFailureKind, EvaluatorRunner, ParsedTurnResponse};
use recovery::{fail_after_thread_error, restart_thread_and_ask};
use selection::{prepare_thread, PreparedThread};
use turn::log_thread_lifecycle_and_ask;

#[derive(Clone, Copy)]
enum ApplicableCurrentModelRetry {
    FreshThreadAfterShortIdMismatch,
    FreshThreadAfterReusedThreadContextWindow,
    FreshThreadAfterTurnTimeout,
    None,
}

pub(crate) fn ask_thread_turn<R: EvaluatorRunner>(
    mut context: ThreadTurnContext<'_, '_, '_, R>,
    request: ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let prepared = prepare_thread(
        context.runtime,
        context.runner,
        context.visible_tree_oid_cache,
        context.interrogation_session,
        &request,
    )?;
    let thread_id = prepared.selection.lifecycle_log.thread_id.clone();
    let response = match log_thread_lifecycle_and_ask(
        &mut context,
        &prepared.selection.lifecycle_log,
        &request,
    ) {
        Ok(response) => response,
        Err(err) => retry_failed_turn(&mut context, &request, &prepared, err, &thread_id)?,
    };
    context
        .interrogation_session
        .thread_state_mut()
        .thread_registry_mut()
        .retire_threads_after_turn(context.runner.take_retired_threads());
    Ok(response)
}

fn retry_failed_turn<R: EvaluatorRunner>(
    context: &mut ThreadTurnContext<'_, '_, '_, R>,
    request: &ThreadTurnRequest<'_>,
    prepared: &PreparedThread,
    err: EvaluatorError,
    thread_id: &str,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let applicable_retry =
        applicable_current_model_retry(err.kind(), prepared.selection.reused_existing_thread);
    match applicable_retry {
        ApplicableCurrentModelRetry::FreshThreadAfterShortIdMismatch => {
            if let Some(progress) = request.progress {
                // Canon `↻`: a fresh-thread retry after a short-ID response error.
                progress.record_fresh_thread_retry_after_short_id_response_error_started();
            }
        }
        ApplicableCurrentModelRetry::FreshThreadAfterReusedThreadContextWindow => {}
        ApplicableCurrentModelRetry::FreshThreadAfterTurnTimeout => {}
        ApplicableCurrentModelRetry::None => {
            return fail_after_thread_error(context.interrogation_session, Some(thread_id), err);
        }
    }
    context.interrogation_session.discard_thread(thread_id);
    restart_thread_and_ask(
        context,
        prepared,
        request,
        &prepared.selection.lifecycle_log,
        err.message_str(),
    )
}

fn applicable_current_model_retry(
    failure_kind: Option<EvaluatorFailureKind>,
    reused_existing_thread: bool,
) -> ApplicableCurrentModelRetry {
    // [Eg,qv] Classify and exhaust an applicable current-model retry before
    // returning a technical failure to the model-fallback loop. A no-progress
    // timeout invalidates its thread but gets one fresh-thread attempt with the
    // same model; failure of that restart returns directly to model fallback.
    match failure_kind {
        // [qv] `ask_once` constructs ShortIdResponse only when the thread has
        // already produced a valid turn. A first-turn mismatch is a normal
        // parsed error response and never enters lifecycle retry handling.
        Some(EvaluatorFailureKind::ShortIdResponse) => {
            ApplicableCurrentModelRetry::FreshThreadAfterShortIdMismatch
        }
        Some(EvaluatorFailureKind::ContextWindow) if reused_existing_thread => {
            ApplicableCurrentModelRetry::FreshThreadAfterReusedThreadContextWindow
        }
        Some(EvaluatorFailureKind::TurnTimeout) => {
            ApplicableCurrentModelRetry::FreshThreadAfterTurnTimeout
        }
        _ => ApplicableCurrentModelRetry::None,
    }
}
