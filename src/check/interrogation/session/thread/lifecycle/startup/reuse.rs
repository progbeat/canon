use super::super::super::state::{
    PrerenderEvaluatorThreadReuseKey, RenderedEvaluatorThreadReuseKey,
};
use super::super::super::ThreadSelection;
use crate::check::interrogation::InterrogationSession;
use crate::evaluator::{EvaluatorError, ThreadEvaluationLogContext, ThreadLifecycleLog};

pub(super) fn reuse_rendered_thread(
    interrogation_session: &mut InterrogationSession,
    prerender_reuse_key: &PrerenderEvaluatorThreadReuseKey,
    rendered_reuse_key: &RenderedEvaluatorThreadReuseKey,
    evaluation_context: ThreadEvaluationLogContext,
    expectation_id: Option<&str>,
) -> Result<Option<ThreadSelection>, EvaluatorError> {
    let existing_thread_id = interrogation_session
        .thread_state()
        .thread_registry()
        .reusable_thread_by_rendered_reuse_key(rendered_reuse_key, expectation_id);
    let Some(existing_thread_id) = existing_thread_id else {
        return Ok(None);
    };
    // This is the second evaluator-thread reuse lookup required by canon:
    // after rendering instructions anyway, reuse a live thread whose rendered
    // base/developer instructions are identical.
    interrogation_session
        .thread_state_mut()
        .thread_registry_mut()
        .bind_prerender_reuse_key_to_thread(
            prerender_reuse_key.clone(),
            existing_thread_id.clone(),
        );
    Ok(Some(ThreadSelection {
        lifecycle_log: thread_reuse_log(
            interrogation_session,
            existing_thread_id,
            evaluation_context,
        )?,
        reused_existing_thread: true,
    }))
}

pub(in crate::check::interrogation::session::thread) fn thread_reuse_log(
    interrogation_session: &InterrogationSession,
    thread_id: String,
    evaluation_context: ThreadEvaluationLogContext,
) -> Result<ThreadLifecycleLog, EvaluatorError> {
    let instructions = interrogation_session
        .thread_state()
        .thread_registry()
        .stored_thread_instructions(&thread_id)
        .ok_or_else(|| {
            EvaluatorError::message(format!(
                "missing instructions for reused thread {thread_id}"
            ))
        })?;
    Ok(ThreadLifecycleLog {
        event: "thread.reuse",
        thread_id,
        base_instructions: instructions.base_instructions,
        developer_instructions: instructions.developer_instructions,
        evaluation_context,
    })
}
