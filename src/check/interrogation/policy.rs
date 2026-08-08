mod q_scope;

use crate::check::core::errors::{
    error_record_from_visible_tree_oid_with_diff_provenance, InterrogationDiffProvenance,
};
use crate::check::core::{
    InterrogationAnswer, InterrogationAnswerData, InterrogationResult, ParsedAnswer,
    ResolvedExpectation, INTERNAL_ERROR_UNPARSABLE,
};
use crate::check::interrogation::session::resolve_diff_from;
use crate::check::interrogation::state::CheckRuntime;
use crate::check::interrogation::{
    interrogate_with_model_fallbacks, InterrogationSession, InterrogationTurnKind,
    ModelFallbackInterrogation, ModelFallbackOutput,
};
use crate::evaluator::{is_interrupted, EvaluatorError, EvaluatorProgress, EvaluatorRunner};
use crate::git::VisibleTreeOidCache;
use crate::logs::DiagnosticLogWriter;
use crate::xpec_state::XpecStateCache;

pub(crate) use q_scope::{
    interrogation_has_passing_answer_for_q_scope_verification,
    q_scope_suggestion_scope_for_independent_verification, write_scope_narrowing_event,
};

// Interrogation Policy implementation map:
// - response schemas for auto-restricted, auto-full-project, fixed-q-scope,
//   and no-hidden-files turns, including `ScopeTooNarrow`, `InvalidQuestion`,
//   `answer`, `evidence`, and `qScopeSuggestion` parsing:
//   `src/check/core/evaluator_response`
// - initial q-scope, no-hide follow-up suppression, invalid qScopeSuggestion
//   rejection, 25%-smaller gate, and reusable-scope verification:
//   `src/check/interrogation/policy/q_scope.rs`
// - check-run `ScopeTooNarrow` retry and q-scope verification sequencing:
//   `src/check/engine/execute/expectation/policy.rs`
// - shared `canon check` and `canon ask` retry and q-scope verification
//   sequencing: `src/check/engine/execute/expectation/policy.rs`
// - evaluator model retry order: `src/check/interrogation/session/model_fallback.rs`
// - configured model-list normalization: `src/evaluator/turn/mod.rs`
// - per-turn thinking, enforced response schema, task input, and thread inputs:
//   `src/check/interrogation/session/thread/answer.rs` and
//   `src/check/interrogation/session/thread/lifecycle/turn.rs`
// - first-turn short-ID mismatch normalization and the typed later-turn
//   mismatch boundary: `src/evaluator/turn/attempt.rs`; forced fresh-thread
//   restart after the typed mismatch:
//   `src/check/interrogation/session/thread/lifecycle`
// A q-scope verification answer becomes final with the proposed scope. Any
// `ScopeTooNarrow` verification may drive one distinct repaired verification;
// if verification still fails, a broader restricted pass remains final, while
// a full-project discovery pass yields the restricted error for a later repair
// instead of becoming a cacheable result.
// A whole-policy audit needs all of these code paths; a narrower q-scope can
// verify only the policy clauses owned by the included files.

pub(crate) struct InterrogationCall<'a> {
    pub(crate) runtime: &'a CheckRuntime<'a>,
    pub(crate) expectation: &'a ResolvedExpectation,
    pub(crate) scope: &'a [String],
    pub(crate) turn_kind: InterrogationTurnKind,
    pub(crate) progress: Option<&'a EvaluatorProgress>,
}

pub(crate) trait RecoverableInterrogation: ModelFallbackOutput {
    fn from_interrogation_error(
        call: &InterrogationCall<'_>,
        error: EvaluatorError,
        xpec_state: &mut XpecStateCache,
        visible_tree_oid_cache: &mut VisibleTreeOidCache,
    ) -> Result<Self, String>;
}

impl RecoverableInterrogation for InterrogationResult {
    fn from_interrogation_error(
        call: &InterrogationCall<'_>,
        error: EvaluatorError,
        xpec_state: &mut XpecStateCache,
        visible_tree_oid_cache: &mut VisibleTreeOidCache,
    ) -> Result<Self, String> {
        let (interrupted, diff_provenance) =
            interrogation_error_context(call, &error, xpec_state, visible_tree_oid_cache)?;
        let visible_tree_oid = call.runtime.visible_tree_oid(
            visible_tree_oid_cache,
            &call.expectation.agent,
            call.scope,
        )?;
        let record = error_record_from_visible_tree_oid_with_diff_provenance(
            call.expectation,
            call.scope,
            error.message_str(),
            visible_tree_oid,
            diff_provenance,
        )?;
        Ok(InterrogationResult::new(record, false, interrupted))
    }
}

impl RecoverableInterrogation for InterrogationAnswer {
    fn from_interrogation_error(
        call: &InterrogationCall<'_>,
        error: EvaluatorError,
        xpec_state: &mut XpecStateCache,
        visible_tree_oid_cache: &mut VisibleTreeOidCache,
    ) -> Result<Self, String> {
        let visible_tree_oid = call.runtime.visible_tree_oid(
            visible_tree_oid_cache,
            &call.expectation.agent,
            call.scope,
        )?;
        let (interrupted, diff_provenance) =
            interrogation_error_context(call, &error, xpec_state, visible_tree_oid_cache)?;
        let (diff_from, diff_from_tree_oid, diff_from_tree_oid_abbrev) =
            InterrogationDiffProvenance::into_optional_record_fields(diff_provenance);
        let mut answer = ParsedAnswer::error_with_evidence(
            INTERNAL_ERROR_UNPARSABLE.to_string(),
            error.to_string(),
        );
        answer.scope = call.scope.to_vec();
        Ok(InterrogationAnswer::new(
            InterrogationAnswerData {
                answer,
                visible_tree_oid,
                diff_from,
                diff_from_tree_oid,
                diff_from_tree_oid_abbrev,
            },
            false,
            interrupted,
        ))
    }
}

pub(crate) fn interrogate_or_error<T: RecoverableInterrogation, R: EvaluatorRunner>(
    call: InterrogationCall<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    interrogation_session: &mut InterrogationSession,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<T, String> {
    match interrogate_with_model_fallbacks(
        ModelFallbackInterrogation {
            runtime: call.runtime,
            expectation: call.expectation,
            enforced_scope: call.scope,
            turn_kind: call.turn_kind,
            progress: call.progress,
        },
        runner,
        diagnostic_log,
        visible_tree_oid_cache,
        interrogation_session,
        xpec_state,
    ) {
        Ok(interrogation) => Ok(interrogation),
        Err(error) => T::from_interrogation_error(&call, error, xpec_state, visible_tree_oid_cache),
    }
}

pub(crate) fn git_backed_interrogation_diff_provenance(
    runtime: &CheckRuntime<'_>,
    expectation: &ResolvedExpectation,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Option<InterrogationDiffProvenance>, String> {
    git_backed_interrogation_diff_provenance_with_cache(
        runtime,
        expectation,
        xpec_state,
        visible_tree_oid_cache,
    )
}

fn git_backed_interrogation_diff_provenance_with_cache(
    runtime: &CheckRuntime<'_>,
    expectation: &ResolvedExpectation,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Option<InterrogationDiffProvenance>, String> {
    if runtime.is_in_place() {
        return Ok(None);
    }
    let last_pass = match expectation.configured_id() {
        Some(_) => xpec_state.read_last_pass(runtime.root, expectation)?,
        None => None,
    };
    let diff_from = resolve_diff_from(
        runtime,
        expectation,
        last_pass.as_ref(),
        visible_tree_oid_cache,
    )
    .map_err(|err| err.to_string())?;
    let diff_from_tree_oid = diff_from
        .tree_oid
        .ok_or("Git-backed interrogation has no diff base tree OID")?;
    let diff_from_tree_oid_abbrev =
        visible_tree_oid_cache.git_oid_abbreviation(runtime.root, &diff_from_tree_oid)?;
    Ok(Some(InterrogationDiffProvenance {
        diff_from: expectation.diff_from.clone(),
        diff_from_tree_oid,
        diff_from_tree_oid_abbrev,
    }))
}

fn interrogation_error_context(
    call: &InterrogationCall<'_>,
    error: &EvaluatorError,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<(bool, Option<InterrogationDiffProvenance>), String> {
    let diff_provenance = git_backed_interrogation_diff_provenance_with_cache(
        call.runtime,
        call.expectation,
        xpec_state,
        visible_tree_oid_cache,
    )?;
    Ok((is_interrupted(error), diff_provenance))
}

#[cfg(test)]
mod tests;
