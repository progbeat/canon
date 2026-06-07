use crate::check::core::types::ParsedAnswer;
use crate::check::interrogation::policy::q_scope_suggestion_should_get_independent_verification;
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::evaluator::EvaluatorError;
use crate::evidence::evidence_file_refs_are_visible_in_root;
use crate::scope::visible_scope;

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
        && narrowed.scope == proposed_scope
        && visible_scope(&runtime.config.agent, &narrowed.scope)
            .map(|scope| {
                evidence_file_refs_are_visible_in_root(&narrowed.evidence, &scope, runtime.root)
            })
            .unwrap_or(false)
}
