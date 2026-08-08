mod create;
mod instructions;
mod reuse;

use super::super::super::InterrogationSession;
use super::super::state::{
    rendered_evaluator_thread_reuse_key, PrerenderEvaluatorThreadReuseKey,
    RenderedEvaluatorThreadReuseKeyContext, ThreadWorkspace,
};
use super::super::{ThreadSelection, ThreadTurnRequest};
use crate::check::interrogation::state::CheckRuntime;
use crate::evaluator::{
    EvaluatorError, EvaluatorRunner, EvaluatorThreadConfigIdentity, ThreadEvaluationLogContext,
};
use create::{start_and_register_thread, NewThread};
use instructions::render_thread_instructions;
use reuse::reuse_rendered_thread;

pub(super) use instructions::{prepare_thread_instructions, PreparedThreadInstructions};
pub(super) use reuse::thread_reuse_log;

pub(super) struct ThreadStartupContext<'ctx> {
    pub(super) workspace: &'ctx ThreadWorkspace,
    pub(super) dynamic_tools: &'ctx [serde_json::Value],
    pub(super) evaluator_config_identity: &'ctx EvaluatorThreadConfigIdentity,
    pub(super) prerender_reuse_key: &'ctx PrerenderEvaluatorThreadReuseKey,
    pub(super) prepared_instructions: &'ctx PreparedThreadInstructions,
    pub(super) interrogation_session: &'ctx mut InterrogationSession,
    pub(super) evaluation_context: ThreadEvaluationLogContext,
}

#[derive(Clone, Copy)]
enum ThreadStartupMode {
    ReuseOrStart,
    StartFresh,
}

pub(super) fn start_or_reuse_thread_after_rendering<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    request: &ThreadTurnRequest<'_>,
    startup_context: ThreadStartupContext<'_>,
) -> Result<ThreadSelection, EvaluatorError> {
    start_thread_after_rendering(
        runtime,
        runner,
        request,
        startup_context,
        ThreadStartupMode::ReuseOrStart,
    )
}

pub(super) fn start_new_thread_after_rendering<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    request: &ThreadTurnRequest<'_>,
    startup_context: ThreadStartupContext<'_>,
) -> Result<ThreadSelection, EvaluatorError> {
    start_thread_after_rendering(
        runtime,
        runner,
        request,
        startup_context,
        ThreadStartupMode::StartFresh,
    )
}

fn start_thread_after_rendering<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    request: &ThreadTurnRequest<'_>,
    startup_context: ThreadStartupContext<'_>,
    mode: ThreadStartupMode,
) -> Result<ThreadSelection, EvaluatorError> {
    let rendered_instructions =
        render_thread_instructions(runtime, request, startup_context.prepared_instructions)?;
    let rendered_reuse_key =
        rendered_evaluator_thread_reuse_key(RenderedEvaluatorThreadReuseKeyContext {
            evaluator_config_identity: startup_context.evaluator_config_identity,
            workspace: startup_context.workspace,
            base_instructions: &rendered_instructions.base_instructions,
            developer_instructions: &rendered_instructions.developer_instructions,
        });
    if matches!(mode, ThreadStartupMode::ReuseOrStart) {
        if let Some(selection) = reuse_rendered_thread(
            startup_context.interrogation_session,
            startup_context.prerender_reuse_key,
            &rendered_reuse_key,
            startup_context.evaluation_context.clone(),
            request.expectation_id,
        )? {
            return Ok(selection);
        }
    }
    let thread_cwd = startup_context.workspace.prepare_cwd(
        runtime,
        startup_context.interrogation_session.thread_state_mut(),
        request,
    )?;
    start_and_register_thread(
        runner,
        startup_context.interrogation_session,
        NewThread {
            request,
            dynamic_tools: startup_context.dynamic_tools,
            prerender_reuse_key: startup_context.prerender_reuse_key,
            rendered_reuse_key,
            thread_cwd,
            evaluation_context: startup_context.evaluation_context,
            rendered_instructions,
        },
    )
}
