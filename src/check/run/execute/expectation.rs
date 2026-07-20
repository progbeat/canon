use super::progress::{start_live_expectation_report, LiveExpectationReport};
use super::CheckRunCaches;
use crate::check::command::output::{
    write_result_output_with_elapsed_timeline, write_result_output_without_started_report,
    SharedCheckOutput,
};
use crate::check::core::errors::error_record_from_visible_tree_oid;
use crate::check::core::{
    CheckOptions, CheckRecord, CheckRecordOutcome, CheckResult, InterrogationAnswer,
    ResolvedExpectation, ERROR_SCOPE_TOO_NARROW,
};
use crate::check::interrogation::policy::{
    git_backed_interrogation_diff_provenance, initial_q_scope_for_fresh_interrogation,
    interrogate_or_error_answer, interrogate_or_error_record, narrowed_scope_is_accepted,
    q_scope_verification_scope_after_initial_pass,
    question_scope_suggestion_scope_for_independent_verification, turn_has_context_compaction,
    write_scope_narrowing_event, InterrogationCall, PolicyInterrogationResult,
};
use crate::check::interrogation::state::{
    should_retry_full_scope_after_error, CheckRuntime, InterrogationRunState,
};
use crate::check::interrogation::{write_expectation_result_event, InterrogationRequestKind};
use crate::config_types::ExpectationTo;
use crate::evaluator::EvaluatorProgress;
use crate::evaluator::EvaluatorRunner;
use crate::hash::full_scope;
use crate::logs::DiagnosticLogWriter;
use crate::platform::check_interrupted;
use crate::scope::scope_is_within;
use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

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

pub(crate) struct TemporaryExpectationInterrogationContext<'a, 'log, R: EvaluatorRunner> {
    pub(crate) runtime: &'a CheckRuntime<'a>,
    pub(crate) runner: &'a mut R,
    pub(crate) diagnostic_log: &'a mut Option<&'log mut DiagnosticLogWriter>,
    pub(crate) caches: &'a mut CheckRunCaches,
    pub(crate) interrogation_run_state: &'a mut InterrogationRunState,
}

pub(crate) fn run_temporary_expectation_interrogation<R: EvaluatorRunner>(
    context: TemporaryExpectationInterrogationContext<'_, '_, R>,
    expectation: &ResolvedExpectation,
    verified_q_scope: &mut Vec<String>,
    progress: Option<&EvaluatorProgress>,
) -> Result<InterrogationAnswer, String> {
    // Temporary ask xpecs share evaluator turns and follow-up policy with
    // normal check xpecs, but return only invocation-local answer data. They do
    // not enter selected-check output, cache reuse, or durable xpec finishing.
    let mut context = TemporaryExpectationRunContext {
        runtime: context.runtime,
        runner: context.runner,
        diagnostic_log: context.diagnostic_log,
        caches: context.caches,
        interrogation_run_state: context.interrogation_run_state,
    };
    run_started_temporary_expectation_interrogation(
        &mut context,
        expectation,
        verified_q_scope,
        progress,
    )
}

struct TemporaryExpectationRunContext<'a, 'log, R: EvaluatorRunner> {
    runtime: &'a CheckRuntime<'a>,
    runner: &'a mut R,
    diagnostic_log: &'a mut Option<&'log mut DiagnosticLogWriter>,
    caches: &'a mut CheckRunCaches,
    interrogation_run_state: &'a mut InterrogationRunState,
}

pub(super) fn run_expectation<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
) -> Result<ExpectationRunOutcome, String> {
    // xpec: k4
    assert!(
        matches!(
            expectation.to,
            ExpectationTo::Agent | ExpectationTo::Caller | ExpectationTo::Shell
        ),
        "xpec evaluator type must exist for every xpec.to value"
    );
    if expectation.to != ExpectationTo::Agent {
        return run_direct_expectation(context, expectation);
    }
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
        // In-place last results intentionally have no Git qScope metadata.
        // That is the interrogation-policy case where no reusable last-pass
        // qScope exists, even though pass/fail history itself is persistent.
        if let Some(scope) = context.runtime.scope_without_reusable_q_scope_history() {
            scope
        } else {
            match initial_q_scope_for_fresh_interrogation(
                context.runtime.root,
                expectation,
                &mut context.caches.xpec_state,
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
        record,
        context_compaction_hit,
        interrupted: interrogation_interrupted,
    } = completed_interrogation;
    assert_final_evaluation_postconditions(expectation, &record);
    finish_user_visible_started_report(started_report, &record);
    // `record_finished_expectation` is the structured report path after public
    // output is finished: returning the completed CheckRecord lets the caller
    // append it to the in-memory CheckRunReport. Both Git-backed and in-place
    // checks update persistent xpec state; the in-place persistence boundary
    // omits Git-only fields from the written last-result record.
    // Later state/cache/logging errors can fail the command, but they occur
    // after the result has been reported through the active report channel.
    record_finished_expectation(
        context,
        expectation,
        &record,
        FinishedRecordSource::Interrogation,
    )?;
    // [7N] The selected-expectation loop has one ordinary stop rule: stop
    // after an evaluated FAIL unless --keep-going was requested. Context
    // compaction invalidates reusable evaluator sessions but does not truncate
    // the selected queue.
    let stop_run = should_stop_after_evaluation(context.options.keep_going, record.passed());
    if context_compaction_hit {
        context.interrogation_run_state.clear_thread_sessions();
    }
    Ok(ExpectationRunOutcome {
        record,
        stop_run,
        interrupted: interrogation_interrupted,
    })
}

fn should_stop_after_evaluation(keep_going: bool, passed: bool) -> bool {
    !keep_going && !passed
}

fn assert_final_evaluation_postconditions(expectation: &ResolvedExpectation, record: &CheckRecord) {
    // xpec: k4
    assert!(
        expectation.expected_answer.is_empty()
            || matches!(record.result, CheckResult::Pass | CheckResult::Fail),
        "an xpec with an expected answer must finish as PASS or FAIL"
    );
    // xpec: k4
    assert!(
        record.error.is_none() || record.result == CheckResult::Fail,
        "an xpec response error must produce FAIL"
    );
}

fn run_direct_expectation<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
) -> Result<ExpectationRunOutcome, String> {
    let scope = full_scope();
    let visible_tree_oid = context.runtime.visible_tree_oid(
        &mut context.caches.visible_tree_oid,
        &expectation.agent,
        &scope,
    )?;
    let started_report = if expectation.to == ExpectationTo::Shell {
        context
            .live_report_output
            .as_ref()
            .map(|output| start_live_expectation_report(None, output, expectation))
            .transpose()?
    } else {
        None
    };
    let evaluation_started_at = Instant::now();
    let response = match expectation.to {
        ExpectationTo::Caller => evaluate_caller(context.result_output, expectation),
        ExpectationTo::Shell => super::shell::evaluate(context.runtime.root, &expectation.question)
            .map(|evaluation| (evaluation.answer, evaluation.transcript)),
        ExpectationTo::Agent => unreachable!("agent xpecs use interrogation"),
    };
    let evaluation_elapsed = evaluation_started_at.elapsed();
    let (observed, evidence, error) = match response {
        Ok((answer, evidence)) => (
            answer,
            (expectation.to == ExpectationTo::Shell).then_some(evidence),
            None,
        ),
        Err(error) => (String::new(), None, Some(error)),
    };
    let result =
        direct_evaluation_result(&expectation.expected_answer, &observed, error.as_deref());
    let record = CheckRecord::current_from_expectation(
        expectation,
        CheckRecordOutcome {
            result,
            observed,
            error,
            evidence,
            scope,
            question_scope_suggestion: None,
            visible_tree_oid,
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
        },
    )?;
    assert_final_evaluation_postconditions(expectation, &record);
    if let Some(started_report) = started_report {
        finish_user_visible_started_report(started_report, &record);
    } else {
        write_user_visible_caller_result(context.result_output, &record, evaluation_elapsed)?;
    }
    record_finished_expectation(
        context,
        expectation,
        &record,
        FinishedRecordSource::DirectEvaluation,
    )?;
    Ok(ExpectationRunOutcome {
        stop_run: should_stop_after_evaluation(context.options.keep_going, record.passed()),
        record,
        interrupted: false,
    })
}

fn direct_evaluation_result(expected: &str, observed: &str, error: Option<&str>) -> CheckResult {
    if error.is_some() {
        CheckResult::Fail
    } else {
        CheckResult::from_expected_answer(expected, observed)
    }
}

fn evaluate_caller(
    result_output: &mut Option<&mut dyn Write>,
    expectation: &ResolvedExpectation,
) -> Result<(String, String), String> {
    if let Some(output) = result_output.as_mut() {
        let prompt = format!(
            "{} ",
            crate::check::command::output::escape_check_output_text(&expectation.question)
        );
        crate::check::command::output::write_stdout_record(
            *output,
            prompt.as_bytes(),
            "caller xpec prompt",
        )?;
    }
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let answer = read_caller_answer(&mut input)?;
    Ok((answer, String::new()))
}

fn read_caller_answer(input: &mut impl BufRead) -> Result<String, String> {
    let mut answer = String::new();
    let bytes_read = input
        .read_line(&mut answer)
        .map_err(|error| format!("failed to read caller answer: {error}"))?;
    if bytes_read == 0 {
        return Err("failed to read caller answer: end of input".to_string());
    }
    trim_line_ending(&mut answer);
    Ok(answer)
}

fn trim_line_ending(line: &mut String) {
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
}

fn record_finished_expectation<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
    record: &CheckRecord,
    source: FinishedRecordSource,
) -> Result<(), String> {
    // This is the result-reporting path used after a CheckRecord is formed.
    // A runtime with persistent history keeps the result available for later
    // inspection. Git-backed selection may also use that state for cache
    // decisions; in-place selection never does.
    // The final call emits the evaluated expectation's expectation.result and,
    // when needed, expectation.review_required events through
    // DiagnosticLogWriter::write_record_event in `src/logs/writer.rs`.
    let Some(persistent_state_root) = context.runtime.persistent_check_state_root() else {
        return write_expectation_result_event(context.diagnostic_log, record);
    };
    if context.runtime.is_in_place() {
        // [I4] In-place has persistent last-result history even though it has
        // no Git tree. This status history supports latest-fail ordering; it
        // cannot define a checkpoint because its serialization omits
        // checkedTreeOid and every other Git-only field.
        context.caches.xpec_state.write_last_result_for_record(
            persistent_state_root,
            context.runtime.git_checked_tree_oid(),
            expectation,
            record,
        )?;
        return write_expectation_result_event(context.diagnostic_log, record);
    }
    match source {
        FinishedRecordSource::Interrogation => {
            context
                .caches
                .xpec_state
                .write_interrogation_last_result_for_record_or_absent_history(
                    Some(persistent_state_root),
                    context.runtime.git_checked_tree_oid(),
                    expectation,
                    record,
                )?;
        }
        FinishedRecordSource::InterrogationAttemptError => {
            let diff_provenance = git_backed_interrogation_diff_provenance(
                context.runtime,
                expectation,
                &mut context.caches.xpec_state,
            )?;
            let mut record_for_state = record.clone();
            if let Some(diff_provenance) = diff_provenance {
                record_for_state.diff_from = Some(diff_provenance.diff_from);
                record_for_state.diff_from_tree_oid = Some(diff_provenance.diff_from_tree_oid);
                record_for_state.diff_from_tree_oid_abbrev =
                    Some(diff_provenance.diff_from_tree_oid_abbrev);
            }
            context
                .caches
                .xpec_state
                .write_interrogation_last_result_for_record_or_absent_history(
                    Some(persistent_state_root),
                    context.runtime.git_checked_tree_oid(),
                    expectation,
                    &record_for_state,
                )?;
        }
        FinishedRecordSource::DirectEvaluation => {
            context
                .caches
                .xpec_state
                .write_last_result_for_record_or_absent_history(
                    Some(persistent_state_root),
                    context.runtime.git_checked_tree_oid(),
                    expectation,
                    record,
                )?;
        }
    }
    write_expectation_result_event(context.diagnostic_log, record)
}

fn assert_final_check_result_has_no_scope_too_narrow(record: &CheckRecord) {
    // xpec: RC
    assert_ne!(
        record.error.as_deref(),
        Some(ERROR_SCOPE_TOO_NARROW),
        "user-visible final check results must not expose ScopeTooNarrow"
    );
}

fn finish_user_visible_started_report(started_report: LiveExpectationReport, record: &CheckRecord) {
    assert_final_check_result_has_no_scope_too_narrow(record);
    started_report.finish_public_output_before_structured_report(record);
}

fn write_user_visible_caller_result(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
    elapsed: Duration,
) -> Result<(), String> {
    assert_final_check_result_has_no_scope_too_narrow(record);
    write_result_output_with_elapsed_timeline(result_output, record, elapsed)
}

fn write_user_visible_result_without_started_report(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
) -> Result<(), String> {
    assert_final_check_result_has_no_scope_too_narrow(record);
    write_result_output_without_started_report(result_output, record)
}

struct CompletedInterrogation {
    record: CheckRecord,
    context_compaction_hit: bool,
    interrupted: bool,
}

#[derive(Clone, Copy)]
enum FinishedRecordSource {
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

fn run_started_temporary_expectation_interrogation<R: EvaluatorRunner>(
    context: &mut TemporaryExpectationRunContext<'_, '_, R>,
    expectation: &ResolvedExpectation,
    verified_q_scope: &mut Vec<String>,
    progress: Option<&EvaluatorProgress>,
) -> Result<InterrogationAnswer, String> {
    let (initial, follow_up_used) = interrogate_temporary_initial_with_full_scope_retry(
        context,
        expectation,
        verified_q_scope,
        progress,
    )?;
    let answer_scope = initial.answer.scope.clone();
    if !scope_is_within(&answer_scope, verified_q_scope) {
        *verified_q_scope = answer_scope;
    }

    // `canon ask` reports an answer-only temporary xpec, so it has no
    // pass/fail result. The `canon check` initial-pass gate is the selected
    // expectation path's `q_scope_verification_scope_after_initial_pass` call.
    let q_scope_verification_scope = if follow_up_used || initial.answer.error.is_some() {
        None
    } else {
        question_scope_suggestion_scope_for_independent_verification(
            context.runtime,
            &expectation.agent,
            initial.answer.question_scope_suggestion.as_deref(),
            verified_q_scope,
            &mut context.caches.visible_tree_oid,
        )?
    };
    let mut interrogation = initial;
    if let Some(proposed_scope) = q_scope_verification_scope {
        let verification_scope = proposed_scope.clone();
        let narrowed = interrogate_or_error_answer(
            InterrogationCall {
                runtime: context.runtime,
                expectation,
                scope: &verification_scope,
                request_kind: InterrogationRequestKind::QScopeVerification,
                progress,
            },
            context.runner,
            context.diagnostic_log,
            context.interrogation_run_state,
            &mut context.caches.xpec_state,
            &mut context.caches.visible_tree_oid,
        )?;
        if narrowed.answer.error.as_deref() == Some(ERROR_SCOPE_TOO_NARROW) {
            if let Some(progress) = progress {
                progress.record_q_scope_verification_returned_scope_too_narrow();
            }
        }
        let accepted = temporary_q_scope_verification_answer_is_accepted(&narrowed);
        write_scope_narrowing_event(
            context.diagnostic_log,
            &expectation.id,
            verified_q_scope,
            &proposed_scope,
            accepted,
        )?;
        if temporary_q_scope_verification_answer_becomes_final(&narrowed) {
            interrogation = narrowed;
        }
    }
    Ok(interrogation)
}

fn interrogate_temporary_initial_with_full_scope_retry<R: EvaluatorRunner>(
    context: &mut TemporaryExpectationRunContext<'_, '_, R>,
    expectation: &ResolvedExpectation,
    verified_q_scope: &mut Vec<String>,
    progress: Option<&EvaluatorProgress>,
) -> Result<(InterrogationAnswer, bool), String> {
    let mut interrogation = interrogate_or_error_answer(
        InterrogationCall {
            runtime: context.runtime,
            expectation,
            scope: verified_q_scope,
            request_kind: InterrogationRequestKind::Initial,
            progress,
        },
        context.runner,
        context.diagnostic_log,
        context.interrogation_run_state,
        &mut context.caches.xpec_state,
        &mut context.caches.visible_tree_oid,
    )?;
    if should_retry_full_scope_after_error(interrogation.answer.error.as_deref(), verified_q_scope)
    {
        *verified_q_scope = full_scope();
        interrogation = interrogate_or_error_answer(
            InterrogationCall {
                runtime: context.runtime,
                expectation,
                scope: verified_q_scope,
                request_kind: InterrogationRequestKind::FullScopeRetry,
                progress,
            },
            context.runner,
            context.diagnostic_log,
            context.interrogation_run_state,
            &mut context.caches.xpec_state,
            &mut context.caches.visible_tree_oid,
        )?;
        return Ok((interrogation, true));
    }
    Ok((interrogation, false))
}

fn run_started_expectation_interrogation<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
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
    let mut context_compaction_hit = turn_has_context_compaction(initial_interrogation);
    let mut interrupted = initial_interrogation.interrupted;

    let record_scope = initial_interrogation.record.scope.clone();
    if !scope_is_within(&record_scope, verified_q_scope) {
        *verified_q_scope = record_scope.clone();
    }
    // xpec: w
    debug_assert!(scope_is_within(&record_scope, verified_q_scope));
    // This is the selected-expectation path's Interrogation Policy q-scope
    // verification follow-up. The only decision made from an evaluator
    // `qScopeSuggestion` is whether this verification follow-up should run;
    // the rest of this function only executes that planned verification turn.
    let q_scope_verification_scope = q_scope_verification_scope_after_initial_pass(
        context.runtime,
        &expectation.agent,
        &initial,
        verified_q_scope,
        &mut context.caches.visible_tree_oid,
    )?;
    let mut interrogation = initial.into_interrogation();
    if let Some(proposed_scope) = q_scope_verification_scope {
        // This verification turn is already the Interrogation Policy's single
        // follow-up for this expectation. It intentionally calls
        // `interrogate_or_error_record` directly instead of the initial-turn
        // full-scope retry helper.
        // Verification ScopeTooNarrow is a concrete rejection of the proposed
        // q-scope, not the final evaluator response for the expectation; the
        // initial answer stays final. Other verification errors remain final
        // human-review results. Pass/fail answers become final.
        let verification_scope = proposed_scope.clone();
        let narrowed = interrogate_or_error_record(
            InterrogationCall {
                runtime: context.runtime,
                expectation,
                scope: &verification_scope,
                request_kind: InterrogationRequestKind::QScopeVerification,
                progress,
            },
            context.runner,
            context.diagnostic_log,
            context.interrogation_run_state,
            &mut context.caches.xpec_state,
            &mut context.caches.visible_tree_oid,
        )?;
        if narrowed.record.error.as_deref() == Some(ERROR_SCOPE_TOO_NARROW) {
            if let Some(progress) = progress {
                progress.record_q_scope_verification_returned_scope_too_narrow();
            }
        }
        context_compaction_hit |= turn_has_context_compaction(&narrowed);
        interrupted |= narrowed.interrupted;
        let accepted = q_scope_verification_result_is_accepted(&narrowed.record);
        write_scope_narrowing_event(
            context.diagnostic_log,
            &expectation.id,
            verified_q_scope,
            &proposed_scope,
            accepted,
        )?;
        if q_scope_verification_result_becomes_final(&narrowed.record) {
            interrogation = narrowed;
        }
    }
    Ok(CompletedInterrogation {
        record: interrogation.record,
        context_compaction_hit,
        interrupted,
    })
}

fn q_scope_verification_result_becomes_final(narrowed: &CheckRecord) -> bool {
    if narrowed.error.as_deref() == Some(ERROR_SCOPE_TOO_NARROW) {
        // The verification result is not final here: it only proves that the
        // proposed narrowed q-scope is too narrow, so the initial answer remains
        // the final evaluator response for the expectation.
        // xpec: w,RC
        assert!(
            !q_scope_verification_result_is_accepted(narrowed),
            "ScopeTooNarrow q-scope verification must not become a user-visible final check result"
        );
        return false;
    }
    true
}

fn q_scope_verification_result_is_accepted(narrowed: &CheckRecord) -> bool {
    narrowed_scope_is_accepted(narrowed)
}

fn temporary_q_scope_verification_answer_becomes_final(narrowed: &InterrogationAnswer) -> bool {
    if narrowed.answer.error.as_deref() == Some(ERROR_SCOPE_TOO_NARROW) {
        // xpec: w
        assert!(
            !temporary_q_scope_verification_answer_is_accepted(narrowed),
            "ScopeTooNarrow q-scope verification must not become a user-visible final query result"
        );
        return false;
    }
    true
}

fn temporary_q_scope_verification_answer_is_accepted(narrowed: &InterrogationAnswer) -> bool {
    narrowed.answer.error.is_none()
}

fn interrogate_initial_with_full_scope_retry<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
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
            request_kind: InterrogationRequestKind::Initial,
            progress,
        },
        context.runner,
        context.diagnostic_log,
        context.interrogation_run_state,
        &mut context.caches.xpec_state,
        &mut context.caches.visible_tree_oid,
    )?;
    let initial_context_compacted = turn_has_context_compaction(&interrogation);
    if should_retry_full_scope_after_error(interrogation.record.error.as_deref(), verified_q_scope)
    {
        // Restricted ScopeTooNarrow is not final. The single policy follow-up
        // retries it once at full scope.
        *verified_q_scope = full_scope();
        interrogation = interrogate_or_error_record(
            InterrogationCall {
                runtime: context.runtime,
                expectation,
                scope: verified_q_scope,
                request_kind: InterrogationRequestKind::FullScopeRetry,
                progress,
            },
            context.runner,
            context.diagnostic_log,
            context.interrogation_run_state,
            &mut context.caches.xpec_state,
            &mut context.caches.visible_tree_oid,
        )?;
        interrogation.context_compacted |= initial_context_compacted;
        return Ok(PolicyInterrogationResult::new(interrogation, true));
    }
    Ok(PolicyInterrogationResult::new(interrogation, false))
}

fn finish_unstarted_expectation_with_error_record<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
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
    expectation: &ResolvedExpectation,
    scope: &[String],
    visible_tree_oid: &Option<String>,
    error: String,
) -> Result<ExpectationRunOutcome, String> {
    let record =
        error_record_from_visible_tree_oid(expectation, scope, &error, visible_tree_oid.clone())?;
    assert_final_evaluation_postconditions(expectation, &record);
    write_user_visible_result_without_started_report(context.result_output, &record)?;
    finish_expectation_with_error_record(
        context,
        expectation,
        record,
        FinishedRecordSource::DirectEvaluation,
    )
}

fn finish_started_expectation_with_error_record<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
    scope: &[String],
    visible_tree_oid: &Option<String>,
    started_report: LiveExpectationReport,
    error: String,
) -> Result<ExpectationRunOutcome, String> {
    let record =
        error_record_from_visible_tree_oid(expectation, scope, &error, visible_tree_oid.clone())?;
    assert_final_evaluation_postconditions(expectation, &record);
    // Public output for this plumbing error does not claim that the error
    // itself came from a prompt diff. The durable last-result write below uses
    // `InterrogationAttemptError` to attach the attempted evaluator prompt
    // diff only to state.
    finish_user_visible_started_report(started_report, &record);
    finish_expectation_with_error_record(
        context,
        expectation,
        record,
        FinishedRecordSource::InterrogationAttemptError,
    )
}

fn finish_expectation_with_error_record<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
    record: CheckRecord,
    source: FinishedRecordSource,
) -> Result<ExpectationRunOutcome, String> {
    record_finished_expectation(context, expectation, &record, source)?;
    context.interrogation_run_state.clear_thread_sessions();
    Ok(ExpectationRunOutcome {
        stop_run: should_stop_after_evaluation(context.options.keep_going, record.passed()),
        record,
        interrupted: false,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        direct_evaluation_result, q_scope_verification_result_becomes_final, read_caller_answer,
        should_stop_after_evaluation, temporary_q_scope_verification_answer_becomes_final,
    };
    use crate::check::core::{
        CheckRecord, CheckResult, InterrogationAnswer, ParsedAnswer, ERROR_INVALID_QUESTION,
        ERROR_SCOPE_TOO_NARROW,
    };
    use crate::hash::full_scope;
    use std::io::Cursor;

    #[test] // xpec: 90,k4
    fn direct_evaluation_error_is_fail_even_in_ask_mode() {
        assert_eq!(
            direct_evaluation_result("", "", Some("direct evaluator failed")),
            CheckResult::Fail
        );
    }

    #[test] // xpec: 90
    fn caller_end_of_input_is_an_evaluation_error() {
        assert_eq!(
            read_caller_answer(&mut Cursor::new(Vec::<u8>::new())),
            Err("failed to read caller answer: end of input".to_string())
        );
    }

    // xpec: 7N
    #[test]
    fn selected_queue_stops_only_after_fail_without_keep_going() {
        assert!(should_stop_after_evaluation(false, false));
        assert!(!should_stop_after_evaluation(false, true));
        assert!(!should_stop_after_evaluation(true, false));
        assert!(!should_stop_after_evaluation(true, true));
    }

    #[test] // xpec: w,RC
    fn q_scope_verification_scope_too_narrow_rejects_scope_without_replacing_initial_result() {
        let narrowed = test_record(CheckResult::Fail, Some(ERROR_SCOPE_TOO_NARROW));

        assert!(!q_scope_verification_result_becomes_final(&narrowed));
    }

    #[test] // xpec: w
    fn q_scope_verification_invalid_question_replaces_initial_result() {
        let narrowed = test_record(CheckResult::Fail, Some(ERROR_INVALID_QUESTION));

        assert!(q_scope_verification_result_becomes_final(&narrowed));
    }

    #[test] // xpec: w
    fn q_scope_verification_answer_result_becomes_final() {
        let fail = test_record(CheckResult::Fail, None);
        let pass = test_record(CheckResult::Pass, None);

        assert!(q_scope_verification_result_becomes_final(&fail));
        assert!(q_scope_verification_result_becomes_final(&pass));
    }

    #[test] // xpec: w
    fn temporary_verification_answer_becomes_final_even_when_observed_changes() {
        let changed = test_answer("no", None);

        assert!(temporary_q_scope_verification_answer_becomes_final(
            &changed
        ));
    }

    #[test] // xpec: w,RC
    fn temporary_scope_too_narrow_verification_does_not_become_final() {
        let narrowed = test_answer("yes", Some(ERROR_SCOPE_TOO_NARROW));

        assert!(!temporary_q_scope_verification_answer_becomes_final(
            &narrowed
        ));
    }

    fn test_record(result: CheckResult, error: Option<&str>) -> CheckRecord {
        test_record_with_observed(result, error.unwrap_or("yes"), error)
    }

    fn test_record_with_observed(
        result: CheckResult,
        observed: &str,
        error: Option<&str>,
    ) -> CheckRecord {
        CheckRecord {
            timestamp: crate::time::format_record_timestamp(0),
            number: 1,
            result,
            to: crate::config_types::ExpectationTo::Agent,
            question: Some("Does it pass?".to_string()),
            expected_answer: Some("yes".to_string()),
            observed: observed.to_string(),
            error: error.map(str::to_string),
            evidence: Some("evidence".to_string()),
            scope: full_scope(),
            question_scope_suggestion: None,
            visible_tree_oid: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
            id: "expectation-id".to_string(),
            display_id: "e".to_string(),
        }
    }

    fn test_answer(observed: &str, error: Option<&str>) -> InterrogationAnswer {
        let mut answer = if let Some(error) = error {
            ParsedAnswer::error(error.to_string(), "evidence".to_string())
        } else {
            ParsedAnswer::answer(observed.to_string(), "evidence".to_string(), None)
        };
        answer.scope = full_scope();
        InterrogationAnswer {
            answer,
            visible_tree_oid: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
            context_compacted: false,
            interrupted: false,
        }
    }
}
