use crate::check::core::ParsedAnswer;
use crate::check::interrogation::policy::question_scope_suggestion_scope_for_independent_verification;
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::evaluator::EvaluatorError;

pub(super) fn scope_for_verification(
    runtime: &CheckRuntime<'_>,
    state: &mut InterrogationRunState,
    enforced_scope: &[String],
    answer: &ParsedAnswer,
) -> Result<Option<Vec<String>>, EvaluatorError> {
    if answer.error.is_some() {
        return Ok(None);
    }
    question_scope_suggestion_scope_for_independent_verification(
        runtime,
        &runtime.config.agent,
        answer.question_scope_suggestion.as_deref(),
        enforced_scope,
        &mut state.visible_tree_oid_cache,
    )
    .map_err(EvaluatorError::from)
}

pub(super) fn answer_is_accepted(
    initial: &ParsedAnswer,
    narrowed: &ParsedAnswer,
    proposed_scope: &[String],
) -> bool {
    narrowed.error.is_none()
        && narrowed.answer == initial.answer
        && narrowed.scope == proposed_scope
}
