use super::super::super::InterrogationSession;
use super::super::state::{
    prerender_evaluator_thread_reuse_key, PrerenderEvaluatorThreadReuseKey,
    PrerenderEvaluatorThreadReuseKeyContext, ThreadWorkspace,
};
use super::super::{ThreadSelection, ThreadTurnRequest};
use super::startup::{
    prepare_thread_instructions, start_or_reuse_thread_after_rendering, thread_reuse_log,
    PreparedThreadInstructions, ThreadStartupContext,
};
use crate::check::interrogation::state::CheckRuntime;
use crate::evaluator::{
    canon_show_dynamic_tools, evaluator_thread_config_identity, EvaluatorError, EvaluatorRunner,
    EvaluatorThreadConfigIdentity, EvaluatorThreadConfigIdentityContext,
};
use crate::git::VisibleTreeOidCache;

pub(super) struct PreparedThread {
    pub(super) workspace: ThreadWorkspace,
    pub(super) dynamic_tools: Vec<serde_json::Value>,
    pub(super) prepared_instructions: PreparedThreadInstructions,
    pub(super) evaluator_config_identity: EvaluatorThreadConfigIdentity,
    pub(super) prerender_reuse_key: PrerenderEvaluatorThreadReuseKey,
    pub(super) selection: ThreadSelection,
}

pub(super) fn prepare_thread<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    interrogation_session: &mut InterrogationSession,
    request: &ThreadTurnRequest<'_>,
) -> Result<PreparedThread, EvaluatorError> {
    let current_visible_tree_oid = runtime
        .visible_tree_oid(
            visible_tree_oid_cache,
            request.agent,
            request.enforced_scope,
        )
        .map_err(EvaluatorError::message)?;
    let workspace = ThreadWorkspace::for_runtime(runtime, current_visible_tree_oid)?;
    let prompt_mode = request.prompt_mode;
    let evaluation_context = workspace.evaluation_log_context(request, prompt_mode)?;
    let prepared_instructions =
        prepare_thread_instructions(runtime, visible_tree_oid_cache, request)?;
    let mut dynamic_tools = runner.evaluator_dynamic_tools()?;
    if request.canon_show_dynamic_tools_enabled() {
        dynamic_tools.extend(canon_show_dynamic_tools().map_err(EvaluatorError::message)?);
    }
    let evaluator_config_identity =
        evaluator_thread_config_identity(EvaluatorThreadConfigIdentityContext {
            agent: request.agent,
            model: request.model,
            thinking: request.thinking,
            dynamic_tools: &dynamic_tools,
        });
    let prerender_reuse_key =
        prerender_evaluator_thread_reuse_key(PrerenderEvaluatorThreadReuseKeyContext {
            evaluator_config_identity: &evaluator_config_identity,
            workspace: &workspace,
            instruction_reuse_key: &prepared_instructions.instruction_reuse_key,
        });
    // A restricted retry, q-scope verification, or different rendered diff
    // transcript misses this pool and starts a separate evaluator thread.
    let existing_thread_id = interrogation_session
        .thread_state()
        .thread_registry()
        .reusable_thread_by_prerender_reuse_key(&prerender_reuse_key, request.expectation_id);
    let selection = match existing_thread_id {
        Some(existing_thread_id) => ThreadSelection {
            lifecycle_log: thread_reuse_log(
                interrogation_session,
                existing_thread_id,
                evaluation_context.clone(),
            )?,
            reused_existing_thread: true,
        },
        None => start_or_reuse_thread_after_rendering(
            runtime,
            runner,
            request,
            ThreadStartupContext {
                workspace: &workspace,
                dynamic_tools: &dynamic_tools,
                evaluator_config_identity: &evaluator_config_identity,
                prerender_reuse_key: &prerender_reuse_key,
                prepared_instructions: &prepared_instructions,
                interrogation_session,
                evaluation_context: evaluation_context.clone(),
            },
        )?,
    };
    Ok(PreparedThread {
        workspace,
        dynamic_tools,
        prepared_instructions,
        evaluator_config_identity,
        prerender_reuse_key,
        selection,
    })
}
