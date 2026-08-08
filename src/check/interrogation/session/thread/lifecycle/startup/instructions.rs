use super::super::super::model::ThreadInstructionReuseKey;
use super::super::super::ThreadTurnRequest;
use crate::check::interrogation::state::CheckRuntime;
use crate::evaluator::{
    developer_instructions_cache_key, evaluator_base_instructions, BaseInstructionsContext,
    DeveloperInstructionsContext, EvaluatorError, RenderedPrompt,
};
use crate::git::VisibleTreeOidCache;
use crate::scope::q_scope_is_full_project;

pub(in crate::check::interrogation::session::thread) struct PreparedThreadInstructions {
    pub(in crate::check::interrogation::session::thread) instruction_reuse_key:
        ThreadInstructionReuseKey,
    num_invisible_files: usize,
}

pub(super) struct RenderedThreadInstructions {
    pub(super) base_instructions: String,
    pub(super) developer_instructions: String,
}

pub(in crate::check::interrogation::session::thread) fn prepare_thread_instructions(
    runtime: &CheckRuntime<'_>,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    request: &ThreadTurnRequest<'_>,
) -> Result<PreparedThreadInstructions, EvaluatorError> {
    let num_invisible_files = runtime.num_invisible_files(
        visible_tree_oid_cache,
        request.agent,
        request.enforced_scope,
    )?;
    let prompt_mode = request.prompt_mode;
    let base_context = BaseInstructionsContext {
        in_place: prompt_mode.git_diff_tree_oids().is_none(),
        q_scope_is_full_project: q_scope_is_full_project(request.enforced_scope),
        q_scope_is_auto: request.response_contract.q_scope_is_auto(),
        q_scope_verification: request.response_contract.is_q_scope_verification(),
    };
    let developer_cache_key = developer_instructions_cache_key(&DeveloperInstructionsContext {
        root: runtime.root,
        mode: prompt_mode,
        question_context: request.question_context,
        visible_scope: request.visible_scope,
        num_invisible_files,
    });
    Ok(PreparedThreadInstructions {
        instruction_reuse_key: ThreadInstructionReuseKey {
            base_context,
            developer_cache_key,
        },
        num_invisible_files,
    })
}

pub(super) fn render_thread_instructions(
    runtime: &CheckRuntime<'_>,
    request: &ThreadTurnRequest<'_>,
    prepared: &PreparedThreadInstructions,
) -> Result<RenderedThreadInstructions, EvaluatorError> {
    // `question_context` is the developer template's xpec.instructions input,
    // not a second prompt or instruction template.
    let rendered_developer_instructions: RenderedPrompt = request
        .prompt_renderer
        .developer_instructions(DeveloperInstructionsContext {
            root: runtime.root,
            mode: request.prompt_mode,
            question_context: request.question_context,
            visible_scope: request.visible_scope,
            num_invisible_files: prepared.num_invisible_files,
        })
        .map_err(EvaluatorError::message)?;
    let developer_instructions = rendered_developer_instructions.text;
    let base_instructions =
        evaluator_base_instructions(prepared.instruction_reuse_key.base_context)
            .map_err(EvaluatorError::message)?;
    Ok(RenderedThreadInstructions {
        base_instructions,
        developer_instructions,
    })
}
