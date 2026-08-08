use super::CheckExpectationRunContext;
use crate::check::core::ResolvedExpectation;
use crate::check::q_scope::{initial_q_scope_for_check_run, initial_q_scope_without_history};
use crate::evaluator::EvaluatorRunner;

pub(super) struct PreparedErrorRecordTree {
    scope: Vec<String>,
    visible_tree_oid: Option<String>,
}

impl PreparedErrorRecordTree {
    pub(super) fn visible_tree_oid_for_scope(&self, scope: &[String]) -> Option<String> {
        (scope == self.scope)
            .then(|| self.visible_tree_oid.clone())
            .flatten()
    }
}

pub(super) fn prepare_unstarted_check_expectation_context<R: EvaluatorRunner>(
    context: &mut CheckExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
) -> Result<(Vec<String>, PreparedErrorRecordTree), String> {
    // In-place last results intentionally have no Git qScope metadata.
    // That is the interrogation-policy case where no reusable last-pass
    // qScope exists, even though pass/fail history itself is persistent.
    let initial_q_scope = if context
        .runtime
        .scope_without_reusable_q_scope_history()
        .is_some()
    {
        initial_q_scope_without_history(expectation)
    } else {
        initial_q_scope_for_check_run(
            context.runtime.root,
            expectation,
            &mut context.caches.xpec_state,
        )?
    };
    // Prepare the tree metadata needed to render an errored expectation before
    // publishing its short-ID report. After `<short ID>.` is visible, later
    // fallible steps can append an ERROR result without doing more fallible
    // tree inspection.
    let visible_tree_oid = context.cached_visible_tree_oid(expectation, &initial_q_scope)?;
    let error_record_tree = PreparedErrorRecordTree {
        scope: initial_q_scope.clone(),
        visible_tree_oid,
    };
    Ok((initial_q_scope, error_record_tree))
}
