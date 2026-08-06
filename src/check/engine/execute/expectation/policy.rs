mod model;
#[cfg(test)]
mod tests;
mod turn;

use super::{
    CheckExpectationRunContext, CompletedCheckInterrogation,
    TemporaryExpectationInterrogationContext,
};
use crate::check::core::{
    evaluate_final_response, InterrogationAnswer, ResolvedExpectation, ERROR_SCOPE_TOO_NARROW,
};
use crate::check::interrogation::InterrogationTurnKind;
use crate::evaluator::{EvaluatorProgress, EvaluatorRunner};
use crate::hash::full_scope;
use crate::scope::scope_is_within;
pub(in crate::check::engine::execute::expectation) use model::PolicyInterrogation;
use model::{PolicyTurnMetadataSource, StartedPolicyInterrogation};
use turn::{PolicyTurnContext, PolicyTurnRunner};

pub(crate) fn run_temporary_expectation_interrogation<R: EvaluatorRunner>(
    context: TemporaryExpectationInterrogationContext<'_, '_, R>,
    expectation: &ResolvedExpectation,
    current_q_scope: &mut Vec<String>,
    progress: Option<&EvaluatorProgress>,
) -> Result<InterrogationAnswer, String> {
    // Temporary ask xpecs share evaluator turns and full-scope retry policy
    // with normal check xpecs, but return only invocation-local answer data.
    // They do not enter selected-check output, cache reuse, or durable xpec
    // finishing.
    let mut context = PolicyTurnContext::<_, InterrogationAnswer>::new(
        context.runtime,
        context.runner,
        context.diagnostic_log,
        context.caches,
        context.interrogation_session,
    );
    let completed =
        run_started_policy_interrogation(&mut context, expectation, current_q_scope, progress)?;
    // [Eg] Temporary ask does not expose or persist evaluation status, but
    // this is its evaluator completion boundary, so enforce evaluate's
    // status and error postconditions before returning invocation-local data.
    evaluate_final_response(
        expectation.expected_answer(),
        &completed.interrogation.output.answer.observed,
        completed.interrogation.output.answer.error.as_deref(),
    );
    Ok(completed.interrogation)
}

const MAX_POLICY_TURNS: usize = 2;

struct CompletedPolicyInterrogation<T> {
    interrogation: T,
    context_compaction_hit: bool,
    interrupted: bool,
}

struct QScopeVerification<T> {
    interrogation: T,
    answer_returned: bool,
}

fn run_started_policy_interrogation<C: PolicyTurnRunner>(
    context: &mut C,
    expectation: &ResolvedExpectation,
    current_q_scope: &mut Vec<String>,
    progress: Option<&EvaluatorProgress>,
) -> Result<CompletedPolicyInterrogation<C::Interrogation>, String>
where
    C::Interrogation: StartedPolicyInterrogation,
{
    let mut interrogation = context.run_policy_turn(
        expectation,
        current_q_scope,
        InterrogationTurnKind::Initial,
        progress,
    )?;
    let mut policy_turn_count = 1;
    let mut accumulated_metadata = interrogation.turn_metadata();
    // xpec: UR,gN
    assert!(
        scope_is_within(interrogation.recorded_q_scope(), current_q_scope),
        "recorded interrogation q-scope must remain within the current q-scope"
    );

    if expectation.q_scope.is_auto() && context.evaluator_turns_may_hide_files() {
        if interrogation.error() == Some(ERROR_SCOPE_TOO_NARROW) {
            // xpec: kg
            assert_ne!(
                current_q_scope,
                &full_scope(),
                "ScopeTooNarrow error on full project scope"
            );
            *current_q_scope = full_scope();
            let mut retry = context.run_policy_turn(
                expectation,
                current_q_scope,
                InterrogationTurnKind::FullScopeRetry,
                progress,
            )?;
            policy_turn_count += 1;
            accumulated_metadata.include(retry.turn_metadata());
            retry.merge_initial_turn_metadata(&interrogation);
            interrogation = retry;
        } else if interrogation.has_passing_answer_for_q_scope_verification() {
            // [5] This is the only check decision that depends on an evaluator
            // qScopeSuggestion. A valid proposal still has to clear the 25%
            // gate before an independent verification turn can use it.
            let proposed_scope = context.q_scope_verification_scope(
                expectation,
                interrogation.q_scope_suggestion(),
                current_q_scope,
            )?;
            if let Some(proposed_scope) = proposed_scope {
                let passing_scope = std::mem::replace(current_q_scope, proposed_scope);
                policy_turn_count += 1;
                let verification = run_q_scope_verification(
                    context,
                    expectation,
                    &passing_scope,
                    current_q_scope,
                    InterrogationTurnKind::QScopeVerification,
                    progress,
                )?;
                let mut narrowed = verification.interrogation;
                accumulated_metadata.include(narrowed.turn_metadata());
                if verification.answer_returned {
                    narrowed.merge_initial_turn_metadata(&interrogation);
                    interrogation = narrowed;
                } else {
                    *current_q_scope = passing_scope;
                }
            }
        }
    }
    assert_at_most_two_policy_turns(policy_turn_count);
    Ok(CompletedPolicyInterrogation {
        interrogation,
        context_compaction_hit: accumulated_metadata.context_compacted,
        interrupted: accumulated_metadata.interrupted,
    })
}

pub(super) fn run_started_check_expectation_interrogation<R: EvaluatorRunner>(
    context: &mut CheckExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
    current_q_scope: &mut Vec<String>,
    progress: Option<&EvaluatorProgress>,
) -> Result<CompletedCheckInterrogation, String> {
    // `run_selected_check_expectation` catches every error from this helper and
    // finishes the already-started report entry with an ERROR record before
    // returning.
    let mut policy_context = PolicyTurnContext::<_, crate::check::core::InterrogationResult>::new(
        context.runtime,
        context.runner,
        context.diagnostic_log,
        context.caches,
        context.interrogation_session,
    );
    let completed = run_started_policy_interrogation(
        &mut policy_context,
        expectation,
        current_q_scope,
        progress,
    )?;
    Ok(CompletedCheckInterrogation {
        record: completed.interrogation.output,
        context_compaction_hit: completed.context_compaction_hit,
        interrupted: completed.interrupted,
    })
}

fn assert_at_most_two_policy_turns(policy_turn_count: usize) {
    // xpec: kg
    assert!(
        policy_turn_count <= MAX_POLICY_TURNS,
        "unexpectedly many turns in interrogation"
    );
}

fn run_q_scope_verification<C: PolicyTurnRunner>(
    context: &mut C,
    expectation: &ResolvedExpectation,
    current_scope: &[String],
    proposed_scope: &[String],
    turn_kind: InterrogationTurnKind,
    progress: Option<&EvaluatorProgress>,
) -> Result<QScopeVerification<C::Interrogation>, String>
where
    C::Interrogation: StartedPolicyInterrogation,
{
    let narrowed = context.run_policy_turn(expectation, proposed_scope, turn_kind, progress)?;
    if narrowed.error() == Some(ERROR_SCOPE_TOO_NARROW) {
        turn_kind.record_scope_too_narrow_progress_marker(progress);
    }
    let answer_returned = narrowed.answer_returned();
    context.write_scope_narrowing(
        expectation.configured_id(),
        current_scope,
        proposed_scope,
        answer_returned,
    )?;
    Ok(QScopeVerification {
        interrogation: narrowed,
        answer_returned,
    })
}
