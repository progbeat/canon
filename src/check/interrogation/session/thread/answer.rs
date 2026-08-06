mod diff_base;

use super::model::evaluator_prompt_mode;
use super::{ask_thread_turn, ThreadTurnContext, ThreadTurnRequest, ThreadTurnResponseContract};
use crate::check::core::InterrogationAnswer;
use crate::check::interrogation::session::model_fallback::{
    ModelAttempt, ModelFallbackInterrogation,
};
use crate::check::interrogation::InterrogationSession;
use crate::config_types::ExpectationTarget;
use crate::evaluator::{
    EvaluatorError, EvaluatorRunner, EvaluatorTurnPromptContext, RenderedPrompt,
};
use crate::git::VisibleTreeOidCache;
use crate::logs::DiagnosticLogWriter;
use crate::scope::sanitize_scope;
use crate::xpec_state::XpecStateCache;

pub(crate) use diff_base::resolve_diff_from;

pub(crate) fn interrogate_expectation_answer_with_model<R: EvaluatorRunner>(
    interrogation: &ModelFallbackInterrogation<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    interrogation_session: &mut InterrogationSession,
    xpec_state: &mut XpecStateCache,
    attempt: ModelAttempt<'_>,
) -> Result<InterrogationAnswer, EvaluatorError> {
    let ModelAttempt {
        model,
        attempt_reason,
        attempt_sequence,
    } = attempt;
    let runtime = interrogation.runtime;
    let expectation = interrogation.expectation;
    // Expectation checks may start from a last-pass restricted scope, but
    // after sanitization this path shares `canon ask`'s first-turn construction:
    // developer instructions and the turn prompt are rendered from
    // `resources/prompts/` plus runtime data.
    let enforced_scope = sanitize_scope(interrogation.enforced_scope)?;
    let in_place = runtime.is_in_place();
    let expectation_id = expectation.configured_id();
    let last_pass = if in_place || expectation_id.is_none() {
        None
    } else {
        xpec_state
            .read_last_pass(runtime.root, expectation)
            .map_err(EvaluatorError::message)?
    };
    let diff_from = resolve_diff_from(
        runtime,
        expectation,
        last_pass.as_ref(),
        visible_tree_oid_cache,
    )?;
    let target_is_diff = matches!(expectation.target.as_ref(), Some(ExpectationTarget::Diff));
    // xpec: 90,X
    assert!(
        !in_place || expectation.target.is_none(),
        "in-place target must be rejected before prompt rendering"
    );
    let prompt_mode =
        evaluator_prompt_mode(runtime, diff_from.tree_oid.as_deref(), target_is_diff)?;
    let visible_scope = runtime
        .visible_scope(&expectation.agent, &enforced_scope)
        .map_err(EvaluatorError::message)?;
    let prompt_renderer = interrogation_session.thread_state().prompt_renderer();
    let rendered_prompt: RenderedPrompt = prompt_renderer
        .evaluator_turn_prompt(EvaluatorTurnPromptContext {
            root: runtime.root,
            short_id: &expectation.display_id,
            question: &expectation.question,
            mode: prompt_mode,
        })
        .map_err(EvaluatorError::message)?;
    let task_input = rendered_prompt.text;
    let thinking = expectation.agent.thinking.as_str();
    let evaluator_attempt = attempt_sequence.next(attempt_reason);
    let response = ask_thread_turn(
        ThreadTurnContext {
            runtime,
            runner,
            diagnostic_log,
            visible_tree_oid_cache,
            interrogation_session,
            xpec_state,
            attempt_sequence,
        },
        ThreadTurnRequest {
            attempt: evaluator_attempt,
            agent: &expectation.agent,
            enforced_scope: &enforced_scope,
            visible_scope: &visible_scope,
            model,
            thinking,
            response_contract: ThreadTurnResponseContract::for_expectation_turn(
                expectation,
                interrogation.turn_kind,
            ),
            diff_target_prior_answer: target_is_diff.then_some(expectation.expected_answer()),
            expectation_id,
            short_id: &expectation.display_id,
            // This is question-scoped canon config data. The implementation-owned
            // evaluator instruction source is the template in `resources/prompts/`;
            // this text is only a value embedded by that source.
            question_context: &expectation.question_context,
            prompt_mode,
            task_input: &task_input,
            prompt_renderer,
            progress: interrogation.progress,
        },
    )?;
    let mut answer = crate::check::interrogation::finalize_interrogation_answer(
        runtime,
        visible_tree_oid_cache,
        &expectation.agent,
        &enforced_scope,
        response.answer,
        response.context_compacted,
    )?;
    if let Some(diff_from_tree_oid) = diff_from.tree_oid {
        let diff_from_tree_oid_abbrev = visible_tree_oid_cache
            .git_oid_abbreviation(runtime.root, &diff_from_tree_oid)
            .map_err(EvaluatorError::message)?;
        answer.output.diff_from = Some(expectation.diff_from.clone());
        answer.output.diff_from_tree_oid = Some(diff_from_tree_oid);
        answer.output.diff_from_tree_oid_abbrev = Some(diff_from_tree_oid_abbrev);
    }
    Ok(answer)
}
