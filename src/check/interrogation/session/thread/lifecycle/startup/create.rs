use super::super::super::state::{
    PrerenderEvaluatorThreadReuseKey, RenderedEvaluatorThreadReuseKey,
};
use super::super::super::{ThreadSelection, ThreadTurnRequest};
use super::super::recovery::fail_after_thread_error;
use super::instructions::RenderedThreadInstructions;
use crate::check::interrogation::InterrogationSession;
use crate::evaluator::{
    EvaluatorError, EvaluatorRunner, ThreadEvaluationLogContext, ThreadLifecycleLog,
};
use std::path::PathBuf;

pub(super) struct NewThread<'a> {
    pub(super) request: &'a ThreadTurnRequest<'a>,
    pub(super) dynamic_tools: &'a [serde_json::Value],
    pub(super) prerender_reuse_key: &'a PrerenderEvaluatorThreadReuseKey,
    pub(super) rendered_reuse_key: RenderedEvaluatorThreadReuseKey,
    pub(super) thread_cwd: PathBuf,
    pub(super) evaluation_context: ThreadEvaluationLogContext,
    pub(super) rendered_instructions: RenderedThreadInstructions,
}

pub(super) fn start_and_register_thread<R: EvaluatorRunner>(
    runner: &mut R,
    interrogation_session: &mut InterrogationSession,
    new_thread: NewThread<'_>,
) -> Result<ThreadSelection, EvaluatorError> {
    // The renderer owns one invocation-local artifact directory. Granting
    // read-only access at thread creation covers artifacts from later turns.
    let template_artifact_directory = new_thread
        .request
        .prompt_renderer
        .artifact_directory()
        .map_err(EvaluatorError::message)?;
    let created_thread_id = match runner.start_thread(
        &new_thread.thread_cwd,
        &template_artifact_directory,
        &new_thread.rendered_instructions.base_instructions,
        &new_thread.rendered_instructions.developer_instructions,
        new_thread.request.agent,
        new_thread.request.model,
        new_thread.request.thinking,
        new_thread.dynamic_tools,
    ) {
        Ok(created_thread_id) => created_thread_id,
        Err(err) => return fail_after_thread_error(interrogation_session, None, err),
    };
    interrogation_session
        .thread_state_mut()
        .thread_registry_mut()
        .register_thread(
            created_thread_id.clone(),
            new_thread.prerender_reuse_key.clone(),
            new_thread.rendered_reuse_key,
            new_thread.rendered_instructions.base_instructions.clone(),
            new_thread
                .rendered_instructions
                .developer_instructions
                .clone(),
        );
    Ok(ThreadSelection {
        lifecycle_log: ThreadLifecycleLog {
            event: "thread.start",
            thread_id: created_thread_id,
            base_instructions: new_thread.rendered_instructions.base_instructions,
            developer_instructions: new_thread.rendered_instructions.developer_instructions,
            evaluation_context: new_thread.evaluation_context,
        },
        reused_existing_thread: false,
    })
}
