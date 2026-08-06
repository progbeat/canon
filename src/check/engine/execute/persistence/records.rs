use super::super::expectation::CheckExpectationRunContext;
use crate::check::core::{CheckRecord, ResolvedExpectation};
use crate::check::interrogation::policy::git_backed_interrogation_diff_provenance;
use crate::evaluator::EvaluatorRunner;
use std::borrow::Cow;

#[derive(Clone, Copy)]
pub(in crate::check::engine::execute) enum FinishedCheckRecordSource {
    // The CheckRecord already came from an evaluator interrogation and carries
    // any prompt-diff provenance that should be visible in stdout and state.
    Interrogation,
    // The public ERROR record was produced by check plumbing while evaluator
    // work was underway. Stdout should not claim the plumbing error used a
    // diff base, but last-result state still records the attempted prompt diff
    // context for the normalized error response.
    InterrogationAttemptError,
    // The error happened before evaluator prompt rendering was attempted.
    DirectEvaluation,
}

pub(in crate::check::engine::execute) fn persist_finished_check_expectation<R: EvaluatorRunner>(
    context: &mut CheckExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
    record: &CheckRecord,
    source: FinishedCheckRecordSource,
) -> Result<(), String> {
    // [g2,Sh] When the runtime exposes a persistent state namespace, this
    // publishes the finished CheckRecord as deliberate, bounded
    // cross-invocation last-result history under XPECS_DIR. Future commands
    // inspect that history and Git-backed selection may use it for cache
    // decisions. Without that namespace, this function skips file output;
    // evaluator threads, per-run caches, and other invocation-local working
    // state are never serialized here.
    let Some(persistent_state_root) = context.runtime.persistent_check_state_root() else {
        return Ok(());
    };
    let checked_tree_oid = context.runtime.git_checked_tree_oid();
    if context.runtime.is_in_place() {
        // [90] In-place has persistent last-result history even though it has
        // no Git tree. This status history supports latest-fail ordering; it
        // cannot define a checkpoint because its serialization omits
        // checkedTreeOid and every other Git-only field.
        return context
            .caches
            .xpec_state
            .write_last_result_for_record(
                persistent_state_root,
                checked_tree_oid,
                expectation,
                record,
            )
            .map(|_| ());
    }
    match source {
        FinishedCheckRecordSource::DirectEvaluation => {
            context
                .caches
                .xpec_state
                .write_last_result_for_record_or_absent_history(
                    Some(persistent_state_root),
                    checked_tree_oid,
                    expectation,
                    record,
                )?;
        }
        FinishedCheckRecordSource::Interrogation
        | FinishedCheckRecordSource::InterrogationAttemptError => {
            let record_for_state = if matches!(source, FinishedCheckRecordSource::Interrogation) {
                Cow::Borrowed(record)
            } else {
                let diff_provenance = git_backed_interrogation_diff_provenance(
                    context.runtime,
                    expectation,
                    &mut context.caches.xpec_state,
                    &mut context.caches.visible_tree_oid_cache,
                )?;
                let mut record_for_state = record.clone();
                if let Some(diff_provenance) = diff_provenance {
                    record_for_state.diff_from = Some(diff_provenance.diff_from);
                    record_for_state.diff_from_tree_oid = Some(diff_provenance.diff_from_tree_oid);
                    record_for_state.diff_from_tree_oid_abbrev =
                        Some(diff_provenance.diff_from_tree_oid_abbrev);
                }
                Cow::Owned(record_for_state)
            };
            context
                .caches
                .xpec_state
                .write_interrogation_last_result_for_record_or_absent_history(
                    Some(persistent_state_root),
                    checked_tree_oid,
                    expectation,
                    &record_for_state,
                )?;
        }
    }
    Ok(())
}
