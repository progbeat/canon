use super::progress::start_live_expectation_report;
use crate::check::core::ResolvedExpectation;
use crate::config_types::ExpectationTo;
use crate::evaluator::EvaluatorRunner;
use crate::hash::full_scope;
use crate::platform::process::check_interrupted;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

const PANICKED_CHECK_EVALUATION_ERROR: &str = "evaluation panicked";

mod direct;
mod finish;
mod model;
mod policy;
mod prepare;

use super::persistence::FinishedCheckRecordSource;
use direct::run_direct_check_expectation;
use finish::{
    append_check_result_to_user_visible_report, finish_started_check_expectation_with_error_record,
    finish_unstarted_check_expectation_with_error_record,
    finish_unstarted_check_expectation_with_error_record_for_visible_tree_oid,
    record_finished_check_expectation,
};
pub(super) use model::CheckExpectationRunContext;
pub(crate) use model::TemporaryExpectationInterrogationContext;
use model::{
    assert_final_check_evaluation_postconditions, CheckExpectationRunOutcome,
    CompletedCheckInterrogation,
};
use policy::run_started_check_expectation_interrogation;
pub(crate) use policy::run_temporary_expectation_interrogation;
use prepare::prepare_unstarted_check_expectation_context;

// [#evaluate]
pub(super) fn run_selected_check_expectation<R: EvaluatorRunner>(
    context: &mut CheckExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
) -> Result<CheckExpectationRunOutcome, String> {
    // xpec: Eg
    assert!(
        matches!(
            expectation.to,
            ExpectationTo::Agent | ExpectationTo::Caller | ExpectationTo::Shell
        ),
        "xpec evaluator type must exist for every xpec.to value"
    );
    if expectation.to != ExpectationTo::Agent {
        return run_direct_check_expectation(context, expectation);
    }
    // Cache hits are resolved before this function is called and never enter
    // this live evaluator path. A short-ID report published here therefore
    // identifies an evaluation in progress. Command-controlled errors and
    // panics append a completed CheckRecord below; after panic finalization the
    // original payload resumes to the command-level panic output.
    // In particular, a same-tree result already handles checked-tree changes
    // outside the stored visible scope before a fresh interrogation can start.
    // Interrupts are checked before any live report starts. Once the short ID
    // is printed it is already the public expectation report; every
    // command-controlled path additionally completes it with a result record.
    if check_interrupted() {
        return finish_unstarted_check_expectation_with_error_record(
            context,
            expectation,
            full_scope(),
            "interrupted".to_string(),
        );
    }

    let (mut current_q_scope, prepared_error_record_tree) =
        match prepare_unstarted_check_expectation_context(context, expectation) {
            Ok(prepared) => prepared,
            Err(error) => {
                return finish_unstarted_check_expectation_with_error_record(
                    context,
                    expectation,
                    full_scope(),
                    error,
                );
            }
        };
    // Start the live report entry before the evaluator turn so the first dot
    // is visible while the expectation is still being evaluated.
    let Some(live_report_output) = context.live_report_output.as_ref() else {
        let visible_tree_oid =
            prepared_error_record_tree.visible_tree_oid_for_scope(&current_q_scope);
        return finish_unstarted_check_expectation_with_error_record_for_visible_tree_oid(
            context,
            expectation,
            &current_q_scope,
            &visible_tree_oid,
            "missing check live report output".to_string(),
        );
    };
    let live_report_state_root = context.runtime.persistent_check_state_root();
    let started_report = match start_live_expectation_report(
        live_report_state_root,
        live_report_output,
        expectation,
    ) {
        Ok(report) => report,
        Err(error) => {
            let visible_tree_oid =
                prepared_error_record_tree.visible_tree_oid_for_scope(&current_q_scope);
            return finish_unstarted_check_expectation_with_error_record_for_visible_tree_oid(
                context,
                expectation,
                &current_q_scope,
                &visible_tree_oid,
                error,
            );
        }
    };
    // Every fallible evaluator step after the short-ID report is published is
    // inside `run_started_check_expectation_interrogation`. Do not route those
    // failures through a cancel-only progress helper: this match converts any
    // post-publication error into the same public ERROR block shape as a normal
    // result.
    let progress = started_report.progress();
    context.runner.set_progress_reporter(Some(progress.clone()));
    let interrogation = catch_unwind(AssertUnwindSafe(|| {
        run_started_check_expectation_interrogation(
            context,
            expectation,
            &mut current_q_scope,
            Some(&progress),
        )
    }));
    let completed_interrogation = match interrogation {
        Err(payload) => {
            // [Eg] BaseException still stops the started evaluator timeline,
            // records FAIL, and reports the synthesized error before the
            // original panic resumes. Independent best-effort boundaries keep
            // a cleanup panic from masking that original payload or preventing
            // the other finalization step.
            let _ = catch_unwind(AssertUnwindSafe(|| {
                context.runner.set_progress_reporter(None)
            }));
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let visible_tree_oid =
                    prepared_error_record_tree.visible_tree_oid_for_scope(&current_q_scope);
                let _ = finish_started_check_expectation_with_error_record(
                    context,
                    expectation,
                    &current_q_scope,
                    &visible_tree_oid,
                    started_report,
                    PANICKED_CHECK_EVALUATION_ERROR.to_string(),
                );
            }));
            resume_unwind(payload)
        }
        Ok(Ok(completed)) => completed,
        Ok(Err(error)) => {
            context.runner.set_progress_reporter(None);
            // A retry or verification turn may have changed the attempted
            // scope after the initial tree metadata was prepared. Never pair
            // that new scope with an OID calculated for the initial scope.
            let visible_tree_oid =
                prepared_error_record_tree.visible_tree_oid_for_scope(&current_q_scope);
            return finish_started_check_expectation_with_error_record(
                context,
                expectation,
                &current_q_scope,
                &visible_tree_oid,
                started_report,
                error.to_string(),
            );
        }
    };
    context.runner.set_progress_reporter(None);
    let CompletedCheckInterrogation {
        record,
        context_compaction_hit,
        interrupted: interrogation_interrupted,
    } = completed_interrogation;
    assert_final_check_evaluation_postconditions(&record);
    append_check_result_to_user_visible_report(started_report, &record);
    context.record_completed(&record);
    // The caller-owned progress report is updated at the public result
    // boundary, before this fallible persistence/logging work. A check runtime
    // that exposes persistent xpec state now updates that state; the in-place
    // persistence boundary omits Git-only fields from the written last-result
    // record. Temporary ask interrogations return before this check-only
    // finishing boundary.
    record_finished_check_expectation(
        context,
        expectation,
        &record,
        FinishedCheckRecordSource::Interrogation,
    )?;
    // [kK] The selected-expectation loop has one ordinary stop rule: stop
    // after an evaluated FAIL unless --keep-going was requested. Context
    // compaction invalidates reusable evaluator threads but does not truncate
    // the selected queue.
    if context_compaction_hit {
        context.interrogation_session.clear_threads();
    }
    Ok(CheckExpectationRunOutcome::after_evaluation(
        &record,
        context.options.keep_going,
        interrogation_interrupted,
    ))
}
