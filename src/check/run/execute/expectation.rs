use super::progress::{start_live_expectation_report, LiveExpectationReport};
use super::CheckRunCaches;
use crate::check::command::output::{
    write_result_output_without_started_report, SharedCheckOutput,
};
use crate::check::core::errors::{error_record_from_visible_tree_oid, INTERNAL_ERROR_UNPARSABLE};
use crate::check::core::{CheckOptions, CheckRecord, SelectedExpectation, ERROR_SCOPE_TOO_NARROW};
use crate::check::interrogation::policy::{
    initial_visible_scope_for_expectation, interrogate_or_error_record, narrowed_scope_is_accepted,
    question_scope_suggestion_scope_for_unused_follow_up, turn_exceeds_break_after_tokens,
    turn_has_context_compaction, write_scope_narrowing_event, InterrogationCall,
    PolicyInterrogationResult,
};
use crate::check::interrogation::state::{
    should_retry_full_scope_after_error, CheckRuntime, InterrogationRunState,
};
use crate::check::interrogation::write_expectation_result_event;
use crate::evaluator::EvaluatorProgress;
use crate::evaluator::EvaluatorRunner;
use crate::hash::full_scope;
use crate::logs::DiagnosticLogWriter;
use crate::platform::check_interrupted;
use crate::scope::scope_is_within;
use std::io::Write;

const FORBIDDEN_FINAL_CHECK_SCOPE_ERROR: &str = "internal error: forbidden final check scope error";

pub(super) struct ExpectationRunContext<'a, 'out, 'log, R: EvaluatorRunner> {
    pub(super) runtime: &'a CheckRuntime<'a>,
    pub(super) options: &'a CheckOptions,
    pub(super) runner: &'a mut R,
    pub(super) diagnostic_log: &'a mut Option<&'log mut DiagnosticLogWriter>,
    pub(super) result_output: &'a mut Option<&'out mut dyn Write>,
    pub(super) live_report_output: &'a Option<SharedCheckOutput>,
    pub(super) caches: &'a mut CheckRunCaches,
    pub(super) interrogation_run_state: &'a mut InterrogationRunState,
}

pub(super) struct ExpectationRunOutcome {
    pub(super) record: CheckRecord,
    pub(super) stop_run: bool,
    pub(super) interrupted: bool,
}

pub(super) fn run_expectation<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &SelectedExpectation,
) -> Result<ExpectationRunOutcome, String> {
    // Cache hits are resolved before this function is called. This path only
    // handles expectations that still need evaluator work, so every report
    // prefix started here belongs to an evaluated expectation and is followed
    // by a completed CheckRecord path; cache hits never enter this live path.
    // In particular, a same-tree result already handles checked-tree changes
    // outside the stored visible scope before a fresh interrogation can start.
    // Interrupts are checked before any live report prefix is started, so this
    // branch cannot leave a printed `<short ID>.` without a completed record.
    if check_interrupted() {
        return finish_unstarted_expectation_with_error_record(
            context,
            expectation,
            full_scope(),
            "interrupted".to_string(),
        );
    }

    let mut verified_q_scope =
        if let Some(scope) = context.runtime.fresh_scope_without_persistent_q_scope() {
            scope
        } else {
            let tree_source = context
                .runtime
                .tree_source()
                .ok_or_else(|| "missing Git tree source".to_string())?;
            match initial_visible_scope_for_expectation(
                context.runtime.root,
                tree_source,
                expectation,
                &mut context.caches.xpec_state,
                &mut context.caches.visible_tree_oid,
            ) {
                Ok(scope) => scope,
                Err(error) => {
                    return finish_unstarted_expectation_with_error_record(
                        context,
                        expectation,
                        full_scope(),
                        error,
                    );
                }
            }
        };
    // Prepare the tree metadata needed to render an errored expectation before
    // printing the live report prefix. After `<short ID>.` is visible, later
    // fallible steps can build an ERROR record without doing more fallible
    // tree inspection.
    let started_report_error_visible_tree_oid = match context.runtime.visible_tree_oid(
        &mut context.caches.visible_tree_oid,
        &expectation.agent,
        &verified_q_scope,
    ) {
        Ok(visible_tree_oid) => visible_tree_oid,
        Err(error) => {
            return finish_unstarted_expectation_with_error_record(
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
        return finish_unstarted_expectation_with_error_record(
            context,
            expectation,
            verified_q_scope,
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
            return finish_unstarted_expectation_with_error_record_for_visible_tree_oid(
                context,
                expectation,
                &verified_q_scope,
                &started_report_error_visible_tree_oid,
                error,
            );
        }
    };
    // Every fallible evaluator step after the report prefix is written is
    // inside `run_started_expectation_interrogation`. Do not route those
    // failures through a cancel-only progress helper: this match converts any
    // post-prefix error into the same public ERROR block shape as a normal
    // result.
    let progress = started_report.progress();
    context.runner.set_progress_reporter(Some(progress.clone()));
    let completed_interrogation = match run_started_expectation_interrogation(
        context,
        expectation,
        &mut verified_q_scope,
        Some(&progress),
    ) {
        Ok(completed) => completed,
        Err(error) => {
            context.runner.set_progress_reporter(None);
            return finish_started_expectation_with_error_record(
                context,
                expectation,
                &verified_q_scope,
                &started_report_error_visible_tree_oid,
                started_report,
                error.to_string(),
            );
        }
    };
    context.runner.set_progress_reporter(None);
    let CompletedInterrogation {
        mut record,
        break_after_tokens_hit,
        context_compaction_hit,
        stop_after_current_expectation,
        interrupted: interrogation_interrupted,
    } = completed_interrogation;
    record = user_visible_final_check_record(record);
    started_report.finish_public_output_or_keep_state_report(&record)?;
    // `record_finished_expectation` is still required after public output:
    // returning the completed CheckRecord lets the caller append it to the
    // in-memory CheckRunReport, while Git-backed runs can also update
    // persistent xpec/live-report state.
    // Later state/cache/logging errors can fail the command, but they occur
    // after the result has been reported through the active report channel.
    record_finished_expectation(context, expectation, &record)?;
    let human_review_required = record.requires_human_review();
    let run_stop_signal_hit =
        break_after_tokens_hit || context_compaction_hit || stop_after_current_expectation;
    let interrupted = run_stop_signal_hit || interrogation_interrupted;
    // Default check order stops after the first evaluated non-pass. Evaluator
    // stop signals are resource/control limits, so they stop after the current
    // result even when the result itself passed.
    let stop_run = run_stop_signal_hit
        || (!context.options.keep_going && (!record.passed() || human_review_required));
    if run_stop_signal_hit {
        context.interrogation_run_state.clear_thread_sessions();
    }
    Ok(ExpectationRunOutcome {
        record,
        stop_run,
        interrupted,
    })
}

fn record_finished_expectation<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
) -> Result<(), String> {
    // This is the durable result-reporting path used after a CheckRecord is
    // formed. Live output has already attempted its human-facing completion;
    // this path keeps the completed result available for later inspection and
    // cache decisions. The final call emits the evaluated expectation's
    // expectation.result and, when needed, expectation.review_required runtime
    // log events through DiagnosticLogWriter::write_record_event.
    context
        .caches
        .xpec_state
        .write_last_result_for_record_or_absent_history(
            context.runtime.persistent_check_state_root(),
            context.runtime.checked_tree_oid(),
            expectation,
            record,
        )?;
    write_expectation_result_event(context.diagnostic_log, record)
}

struct CompletedInterrogation {
    record: CheckRecord,
    break_after_tokens_hit: bool,
    context_compaction_hit: bool,
    stop_after_current_expectation: bool,
    interrupted: bool,
}

fn run_started_expectation_interrogation<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &SelectedExpectation,
    verified_q_scope: &mut Vec<String>,
    progress: Option<&EvaluatorProgress>,
) -> Result<CompletedInterrogation, String> {
    // `run_expectation` catches every error from this helper and finishes the
    // already-started report entry with an ERROR record before returning.
    let initial = interrogate_initial_with_full_scope_retry(
        context,
        expectation,
        verified_q_scope,
        progress,
    )?;
    let initial_interrogation = initial.interrogation();
    let mut break_after_tokens_hit =
        turn_exceeds_break_after_tokens(initial_interrogation, context.options.break_after_tokens);
    let mut context_compaction_hit = turn_has_context_compaction(initial_interrogation);
    let mut stop_after_current_expectation = initial_interrogation.stop_after_current_expectation;
    let mut interrupted = initial_interrogation.interrupted;

    let record_scope = initial_interrogation.record.scope.clone();
    if !scope_is_within(&record_scope, verified_q_scope) {
        *verified_q_scope = record_scope.clone();
    }
    debug_assert!(scope_is_within(&record_scope, verified_q_scope));
    let initial_result = initial_interrogation.record.result;
    // This is the selected-expectation path's Interrogation Policy q-scope
    // verification follow-up. The only decision made from an evaluator
    // `qScopeSuggestion` is whether this verification follow-up should run;
    // the rest of this function only executes that planned verification turn.
    let q_scope_verification_scope = question_scope_suggestion_scope_for_unused_follow_up(
        context.runtime,
        &expectation.agent,
        &initial,
        verified_q_scope,
        &mut context.caches.visible_tree_oid,
    )?;
    let mut interrogation = initial.into_interrogation();
    if let Some(proposed_scope) = q_scope_verification_scope {
        if let Some(progress) = progress {
            // A narrowed verification may need a fresh evaluator session for
            // its visible scope, so record the canon `↘` marker before any
            // resulting thread/start control message.
            progress.record_q_scope_verification_started();
        }
        // This verification turn is already the Interrogation Policy's single
        // follow-up for this expectation. It intentionally calls
        // `interrogate_or_error_record` directly instead of the initial-turn
        // full-scope retry helper.
        // Verification ScopeTooNarrow is a concrete rejection of the proposed
        // q-scope, not the final evaluator response for the expectation; the
        // initial answer stays final. Other verification errors remain final
        // human-review results. Pass/fail results use the acceptance matrix
        // below.
        let verification_scope = proposed_scope.clone();
        let narrowed = interrogate_or_error_record(
            InterrogationCall {
                runtime: context.runtime,
                expectation,
                scope: &verification_scope,
                progress,
            },
            context.runner,
            context.diagnostic_log,
            context.interrogation_run_state,
            &mut context.caches.xpec_state,
            &mut context.caches.visible_tree_oid,
        )?;
        break_after_tokens_hit |=
            turn_exceeds_break_after_tokens(&narrowed, context.options.break_after_tokens);
        context_compaction_hit |= turn_has_context_compaction(&narrowed);
        stop_after_current_expectation |= narrowed.stop_after_current_expectation;
        interrupted |= narrowed.interrupted;
        let accepted = narrowed_scope_is_accepted(initial_result, &narrowed.record);
        write_scope_narrowing_event(
            context.diagnostic_log,
            &expectation.id,
            verified_q_scope,
            &proposed_scope,
            accepted,
        )?;
        if q_scope_verification_record_replaces_initial(initial_result, &narrowed.record) {
            interrogation = narrowed;
        }
    }
    Ok(CompletedInterrogation {
        record: interrogation.record,
        break_after_tokens_hit,
        context_compaction_hit,
        stop_after_current_expectation,
        interrupted,
    })
}

fn q_scope_verification_record_replaces_initial(
    initial_result: crate::check::core::CheckResult,
    narrowed: &CheckRecord,
) -> bool {
    if narrowed.error.as_deref() == Some(ERROR_SCOPE_TOO_NARROW) {
        return false;
    }
    narrowed.error.is_some() || narrowed_scope_is_accepted(initial_result, narrowed)
}

fn interrogate_initial_with_full_scope_retry<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &SelectedExpectation,
    verified_q_scope: &mut Vec<String>,
    progress: Option<&EvaluatorProgress>,
) -> Result<PolicyInterrogationResult, String> {
    // This helper is only for the initial interrogation. Its retry consumes
    // the one follow-up turn allowed by Interrogation Policy, so q-scope
    // verification must not call back into it.
    let mut interrogation = interrogate_or_error_record(
        InterrogationCall {
            runtime: context.runtime,
            expectation,
            scope: verified_q_scope,
            progress,
        },
        context.runner,
        context.diagnostic_log,
        context.interrogation_run_state,
        &mut context.caches.xpec_state,
        &mut context.caches.visible_tree_oid,
    )?;
    let should_stop_after_current_expectation =
        turn_exceeds_break_after_tokens(&interrogation, context.options.break_after_tokens)
            || turn_has_context_compaction(&interrogation);
    if should_retry_full_scope_after_error(interrogation.record.error.as_deref(), verified_q_scope)
    {
        // Restricted ScopeTooNarrow is not final. The single policy follow-up
        // retries it once at full scope.
        if let Some(progress) = progress {
            // A full-scope retry may need a fresh evaluator session for its
            // visible scope, so record the canon `↗` marker before any
            // resulting thread/start control message.
            progress.record_full_scope_retry_started();
        }
        *verified_q_scope = full_scope();
        interrogation = interrogate_or_error_record(
            InterrogationCall {
                runtime: context.runtime,
                expectation,
                scope: verified_q_scope,
                progress,
            },
            context.runner,
            context.diagnostic_log,
            context.interrogation_run_state,
            &mut context.caches.xpec_state,
            &mut context.caches.visible_tree_oid,
        )?;
        interrogation.stop_after_current_expectation |= should_stop_after_current_expectation;
        return Ok(PolicyInterrogationResult::new(interrogation, true));
    }
    Ok(PolicyInterrogationResult::new(interrogation, false))
}

fn finish_unstarted_expectation_with_error_record<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &SelectedExpectation,
    scope: Vec<String>,
    error: String,
) -> Result<ExpectationRunOutcome, String> {
    let visible_tree_oid = context.runtime.visible_tree_oid(
        &mut context.caches.visible_tree_oid,
        &expectation.agent,
        &scope,
    )?;
    finish_unstarted_expectation_with_error_record_for_visible_tree_oid(
        context,
        expectation,
        &scope,
        &visible_tree_oid,
        error,
    )
}

fn finish_unstarted_expectation_with_error_record_for_visible_tree_oid<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &SelectedExpectation,
    scope: &[String],
    visible_tree_oid: &str,
    error: String,
) -> Result<ExpectationRunOutcome, String> {
    let record = user_visible_final_check_record(error_record_from_visible_tree_oid(
        expectation,
        scope,
        &error,
        visible_tree_oid.to_string(),
    )?);
    write_result_output_without_started_report(context.result_output, &record)?;
    finish_expectation_with_error_record(context, expectation, record)
}

fn finish_started_expectation_with_error_record<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &SelectedExpectation,
    scope: &[String],
    visible_tree_oid: &str,
    started_report: LiveExpectationReport,
    error: String,
) -> Result<ExpectationRunOutcome, String> {
    let record = user_visible_final_check_record(error_record_from_visible_tree_oid(
        expectation,
        scope,
        &error,
        visible_tree_oid.to_string(),
    )?);
    started_report.finish_public_output_or_keep_state_report(&record)?;
    finish_expectation_with_error_record(context, expectation, record)
}

fn user_visible_final_check_record(mut record: CheckRecord) -> CheckRecord {
    if assert_final_check_record_has_no_scope_too_narrow(&record).is_ok() {
        return record;
    }
    record.observed = INTERNAL_ERROR_UNPARSABLE.to_string();
    record.error = Some(INTERNAL_ERROR_UNPARSABLE.to_string());
    record.evidence = FORBIDDEN_FINAL_CHECK_SCOPE_ERROR.to_string();
    record.question_scope_suggestion = None;
    record
}

fn assert_final_check_record_has_no_scope_too_narrow(record: &CheckRecord) -> Result<(), String> {
    if record.error.as_deref() == Some(ERROR_SCOPE_TOO_NARROW) {
        Err(FORBIDDEN_FINAL_CHECK_SCOPE_ERROR.to_string())
    } else {
        Ok(())
    }
}

fn finish_expectation_with_error_record<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &SelectedExpectation,
    record: CheckRecord,
) -> Result<ExpectationRunOutcome, String> {
    record_finished_expectation(context, expectation, &record)?;
    context.interrogation_run_state.clear_thread_sessions();
    Ok(ExpectationRunOutcome {
        record,
        stop_run: !context.options.keep_going,
        interrupted: false,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        q_scope_verification_record_replaces_initial, user_visible_final_check_record,
        FORBIDDEN_FINAL_CHECK_SCOPE_ERROR,
    };
    use crate::check::core::errors::INTERNAL_ERROR_UNPARSABLE;
    use crate::check::core::{
        CheckRecord, CheckResult, ERROR_INVALID_QUESTION, ERROR_SCOPE_TOO_NARROW,
    };
    use crate::hash::full_scope;

    #[test]
    fn q_scope_verification_scope_too_narrow_rejects_scope_without_replacing_initial_result() {
        let narrowed = test_record(CheckResult::Fail, Some(ERROR_SCOPE_TOO_NARROW));

        assert!(!q_scope_verification_record_replaces_initial(
            CheckResult::Pass,
            &narrowed
        ));
    }

    #[test]
    fn q_scope_verification_invalid_question_replaces_initial_result() {
        let narrowed = test_record(CheckResult::Fail, Some(ERROR_INVALID_QUESTION));

        assert!(q_scope_verification_record_replaces_initial(
            CheckResult::Pass,
            &narrowed
        ));
    }

    #[test]
    fn q_scope_verification_pass_fail_matrix_still_applies_without_error() {
        let fail = test_record(CheckResult::Fail, None);
        let pass = test_record(CheckResult::Pass, None);

        assert!(q_scope_verification_record_replaces_initial(
            CheckResult::Pass,
            &fail
        ));
        assert!(!q_scope_verification_record_replaces_initial(
            CheckResult::Fail,
            &pass
        ));
    }

    #[test]
    fn final_check_record_replaces_forbidden_scope_too_narrow_error() {
        let mut record = test_record(CheckResult::Fail, Some(ERROR_SCOPE_TOO_NARROW));
        record.question_scope_suggestion = Some(vec!["src".to_string()]);

        let final_record = user_visible_final_check_record(record);

        assert_eq!(final_record.observed, INTERNAL_ERROR_UNPARSABLE);
        assert_eq!(
            final_record.error.as_deref(),
            Some(INTERNAL_ERROR_UNPARSABLE)
        );
        assert_eq!(final_record.evidence, FORBIDDEN_FINAL_CHECK_SCOPE_ERROR);
        assert!(final_record.question_scope_suggestion.is_none());
    }

    fn test_record(result: CheckResult, error: Option<&str>) -> CheckRecord {
        CheckRecord {
            timestamp: crate::time::format_record_timestamp(0),
            number: 1,
            result,
            question: Some("Does it pass?".to_string()),
            expected_answer: Some("yes".to_string()),
            observed: error.unwrap_or("yes").to_string(),
            error: error.map(str::to_string),
            evidence: "evidence".to_string(),
            scope: full_scope(),
            question_scope_suggestion: None,
            visible_tree_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            id: "expectation-id".to_string(),
            display_id: "e".to_string(),
        }
    }
}
