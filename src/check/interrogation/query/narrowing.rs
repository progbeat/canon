use crate::check::core::types::ParsedAnswer;
use crate::check::interrogation::policy::{
    q_scope_suggestion_should_get_independent_verification, verified_q_scope_evidence_is_accepted,
};
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::evaluator::EvaluatorError;

pub(super) fn should_verify(
    runtime: &CheckRuntime<'_>,
    state: &mut InterrogationRunState,
    enforced_scope: &[String],
    answer: &ParsedAnswer,
) -> Result<bool, EvaluatorError> {
    if answer.error.is_some() {
        return Ok(false);
    }
    q_scope_suggestion_should_get_independent_verification(
        runtime,
        &runtime.config.agent,
        answer.q_scope_suggestion.as_deref(),
        enforced_scope,
        &mut state.visible_tree_oid_cache,
    )
    .map_err(EvaluatorError::from)
}

pub(super) fn answer_is_accepted(
    runtime: &CheckRuntime<'_>,
    narrowed: &ParsedAnswer,
    proposed_scope: &[String],
) -> bool {
    narrowed.error.is_none()
        && verified_q_scope_evidence_is_accepted(
            runtime.root,
            &runtime.config.agent,
            &narrowed.scope,
            &narrowed.evidence,
            proposed_scope,
        )
}
