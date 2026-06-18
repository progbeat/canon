use super::progress::{start_live_expectation_report, StateBackedLiveExpectationReport};
use super::CheckRunCaches;
use crate::check::command::output::{
    write_result_output_without_started_report, SharedCheckOutput,
};
use crate::check::core::errors::error_record_from_visible_tree_oid_at;
use crate::check::core::{CheckOptions, CheckRecord, SelectedExpectation};
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
use crate::time::{format_record_timestamp, unix_timestamp};
use std::io::Write;

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

    let mut verified_q_scope = match initial_visible_scope_for_expectation(
        context.runtime.root,
        context.runtime.tree_source,
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
    };
    // Prepare the metadata needed to render an errored expectation before
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
    let started_report_error_timestamp = match unix_timestamp().map(format_record_timestamp) {
        Ok(timestamp) => timestamp,
        Err(error) => {
            return finish_unstarted_expectation_with_error_record(
                context,
                expectation,
                verified_q_scope.clone(),
                error,
            );
        }
    };
    // Start the live report entry before the evaluator turn so the first dot
    // is visible while the expectation is still being evaluated.
    let mut started_report = match start_live_expectation_report(
        context.runtime.root,
        context.live_report_output,
        expectation,
    ) {
        Ok(report) => report,
        Err(error) => {
            let mut started_report = None;
            return finish_started_expectation_with_error_record(
                context,
                expectation,
                &verified_q_scope,
                &started_report_error_visible_tree_oid,
                &started_report_error_timestamp,
                &mut started_report,
                error,
            );
        }
    };
    // Every fallible evaluator step after the report prefix is written is
    // inside `run_started_expectation_interrogation`. Do not route those
    // failures through a cancel-only progress helper: this match converts any
    // post-prefix error into the same public ERROR block shape as a normal
    // result.
    let progress = started_report.as_ref().map(|report| report.progress());
    context.runner.set_progress_reporter(progress.clone());
    let completed_interrogation = match run_started_expectation_interrogation(
        context,
        expectation,
        &mut verified_q_scope,
        progress.as_ref(),
    ) {
        Ok(completed) => completed,
        Err(error) => {
            context.runner.set_progress_reporter(None);
            return finish_started_expectation_with_error_record(
                context,
                expectation,
                &verified_q_scope,
                &started_report_error_visible_tree_oid,
                &started_report_error_timestamp,
                &mut started_report,
                error.to_string(),
            );
        }
    };
    context.runner.set_progress_reporter(None);
    let CompletedInterrogation {
        record,
        break_after_tokens_hit,
        context_compaction_hit,
        stop_after_current_expectation,
        interrupted: interrogation_interrupted,
    } = completed_interrogation;
    if let Some(report) = started_report.take() {
        // Live output is best-effort after the prefix; completion does not
        // create a return path that can drop the completed CheckRecord.
        report.finish_public_output_or_keep_state_report(&record);
    } else {
        write_result_output_without_started_report(context.result_output, &record)?;
    }
    // From this point on, the public per-expectation result has already been
    // written. Later state/cache/logging errors can fail the command, but they
    // occur after the visible result block.
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
    // cache decisions.
    context.caches.xpec_state.write_last_result_for_record(
        context.runtime.root,
        &context.runtime.tree_context.checked_tree_oid,
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
    let initial =
        interrogate_with_full_scope_retry(context, expectation, verified_q_scope, progress)?;
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
    // This is the Interrogation Policy q-scope verification follow-up. It is
    // the only check-run decision point that consumes an evaluator
    // `qScopeSuggestion`; the rest of this function only executes the
    // verification turn planned by policy. It is unrelated to check-config
    // expectation item expansion.
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
            progress.record_q_scope_verification_started();
        }
        let verification_scope = proposed_scope.clone();
        let narrowed = interrogate_or_error_record(
            InterrogationCall {
                runtime: context.runtime,
                expectation,
                scope: &verification_scope,
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
        if accepted {
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

fn interrogate_with_full_scope_retry<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &SelectedExpectation,
    verified_q_scope: &mut Vec<String>,
    progress: Option<&EvaluatorProgress>,
) -> Result<PolicyInterrogationResult, String> {
    let mut interrogation = interrogate_or_error_record(
        InterrogationCall {
            runtime: context.runtime,
            expectation,
            scope: verified_q_scope,
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
            progress.record_full_scope_retry_started();
        }
        *verified_q_scope = full_scope();
        interrogation = interrogate_or_error_record(
            InterrogationCall {
                runtime: context.runtime,
                expectation,
                scope: verified_q_scope,
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
    let timestamp = format_record_timestamp(unix_timestamp()?);
    let mut started_report = None;
    finish_started_expectation_with_error_record(
        context,
        expectation,
        &scope,
        &visible_tree_oid,
        &timestamp,
        &mut started_report,
        error,
    )
}

fn finish_started_expectation_with_error_record<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &SelectedExpectation,
    scope: &[String],
    visible_tree_oid: &str,
    timestamp: &str,
    started_report: &mut Option<StateBackedLiveExpectationReport>,
    error: String,
) -> Result<ExpectationRunOutcome, String> {
    let record = error_record_from_visible_tree_oid_at(
        expectation,
        scope,
        &error,
        visible_tree_oid.to_string(),
        timestamp.to_string(),
    );
    if let Some(report) = started_report.take() {
        report.finish_public_output_or_keep_state_report(&record);
    } else {
        write_result_output_without_started_report(context.result_output, &record)?;
    }
    record_finished_expectation(context, expectation, &record)?;
    context.interrogation_run_state.clear_thread_sessions();
    Ok(ExpectationRunOutcome {
        record,
        stop_run: !context.options.keep_going,
        interrupted: false,
    })
}
