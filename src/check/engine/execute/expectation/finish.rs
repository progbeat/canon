mod output;

use super::{
    assert_final_check_evaluation_postconditions, CheckExpectationRunContext,
    CheckExpectationRunOutcome,
};
use crate::check::core::errors::error_record_from_visible_tree_oid;
use crate::check::core::{CheckRecord, ResolvedExpectation};
use crate::check::engine::execute::persistence::{
    persist_finished_check_expectation, FinishedCheckRecordSource,
};
use crate::check::interrogation::write_expectation_result_event;
use crate::evaluator::EvaluatorRunner;
pub(super) use output::{
    append_check_result_to_user_visible_report, write_user_visible_caller_check_result,
    write_user_visible_check_result_without_started_report,
};
pub(super) fn finish_unstarted_check_expectation_with_error_record<R: EvaluatorRunner>(
    context: &mut CheckExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
    scope: Vec<String>,
    error: String,
) -> Result<CheckExpectationRunOutcome, String> {
    let visible_tree_oid = context.cached_visible_tree_oid(expectation, &scope)?;
    finish_unstarted_check_expectation_with_error_record_for_visible_tree_oid(
        context,
        expectation,
        &scope,
        &visible_tree_oid,
        error,
    )
}

pub(super) fn finish_unstarted_check_expectation_with_error_record_for_visible_tree_oid<
    R: EvaluatorRunner,
>(
    context: &mut CheckExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
    scope: &[String],
    visible_tree_oid: &Option<String>,
    error: String,
) -> Result<CheckExpectationRunOutcome, String> {
    let record = final_check_error_record(expectation, scope, visible_tree_oid, &error)?;
    write_user_visible_check_result_without_started_report(context.result_output, &record)?;
    finish_check_expectation_with_error_record(
        context,
        expectation,
        record,
        FinishedCheckRecordSource::DirectEvaluation,
    )
}

pub(super) fn finish_started_check_expectation_with_error_record<R: EvaluatorRunner>(
    context: &mut CheckExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
    scope: &[String],
    visible_tree_oid: &Option<String>,
    started_report: super::super::progress::LiveExpectationReport,
    error: String,
) -> Result<CheckExpectationRunOutcome, String> {
    let record = final_check_error_record(expectation, scope, visible_tree_oid, &error)?;
    // Public output for this plumbing error does not claim that the error
    // itself came from a prompt diff. The durable last-result write below uses
    // `InterrogationAttemptError` to attach the attempted evaluator prompt
    // diff only to state.
    append_check_result_to_user_visible_report(started_report, &record);
    finish_check_expectation_with_error_record(
        context,
        expectation,
        record,
        FinishedCheckRecordSource::InterrogationAttemptError,
    )
}

fn finish_check_expectation_with_error_record<R: EvaluatorRunner>(
    context: &mut CheckExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
    record: CheckRecord,
    source: FinishedCheckRecordSource,
) -> Result<CheckExpectationRunOutcome, String> {
    context.record_completed(&record);
    record_finished_check_expectation(context, expectation, &record, source)?;
    context.interrogation_session.clear_threads();
    Ok(CheckExpectationRunOutcome::after_evaluation(
        &record,
        context.options.keep_going,
        false,
    ))
}

pub(super) fn record_finished_check_expectation<R: EvaluatorRunner>(
    context: &mut CheckExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
    record: &CheckRecord,
    source: FinishedCheckRecordSource,
) -> Result<(), String> {
    // [w] This is the terminal runtime-log boundary for every finished check
    // record, including evaluator parse failures normalized into human-review
    // records. `write_expectation_result_event` emits `expectation.result` and
    // additionally `expectation.review_required` when the record has a review
    // reason. Last-result state and those outcome events are independent
    // completed-record side effects, so attempt both even if one fails.
    let persistence_result =
        persist_finished_check_expectation(context, expectation, record, source);
    let runtime_log_result = write_expectation_result_event(context.diagnostic_log, record);
    match (persistence_result, runtime_log_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(persistence_error), Err(log_error)) => Err(format!(
            "{persistence_error}; also failed to write expectation runtime log: {log_error}"
        )),
    }
}

fn final_check_error_record(
    expectation: &ResolvedExpectation,
    scope: &[String],
    visible_tree_oid: &Option<String>,
    error: &str,
) -> Result<CheckRecord, String> {
    let record =
        error_record_from_visible_tree_oid(expectation, scope, error, visible_tree_oid.clone())?;
    assert_final_check_evaluation_postconditions(&record);
    Ok(record)
}
