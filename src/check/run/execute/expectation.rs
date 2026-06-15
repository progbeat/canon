use super::progress::{start_live_expectation_report, StateBackedLiveExpectationReport};
use super::CheckRunCaches;
use crate::check::command::output::{
    write_result_output_without_started_report, SharedCheckOutput,
};
use crate::check::core::errors::error_record_from_visible_tree_oid_at;
use crate::check::core::{CheckOptions, CheckRecord, SelectedExpectation};
use crate::check::interrogation::policy::{
    interrogate_with_full_scope_retry, narrowed_scope_is_accepted,
    question_scope_suggestion_scope_for_independent_verification, turn_exceeds_break_after_tokens,
    turn_has_context_compaction, write_scope_narrowing_event, ScopedInterrogation,
};
use crate::check::interrogation::state::{
    initial_visible_scope_for_expectation, CheckRuntime, InterrogationRunState,
};
use crate::check::interrogation::write_expectation_result_event;
use crate::evaluator::EvaluatorRunner;
use crate::logs::DiagnosticLogWriter;
use crate::platform::check_interrupted;
use crate::scope::scope_is_within;
use crate::time::{format_record_timestamp, unix_timestamp};
use std::collections::BTreeSet;
use std::io::Write;

pub(super) struct ExpectationRunContext<'a, 'out, 'log, R: EvaluatorRunner> {
    pub(super) runtime: &'a CheckRuntime<'a>,
    pub(super) options: &'a CheckOptions,
    pub(super) active_lazy_full_scope_reset_ids: &'a BTreeSet<String>,
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
}

pub(super) fn run_expectation<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &SelectedExpectation,
) -> Result<ExpectationRunOutcome, String> {
    // Cache hits are resolved before this function is called. This path only
    // handles expectations that still need evaluator work, so every report
    // prefix started here belongs to an evaluated expectation and is followed
    // by a completed CheckRecord path; cache hits never enter this live path.
    macro_rules! return_expectation_error {
        ($error:expr) => {{
            let error = $error.to_string();
            return Err(error);
        }};
    }
    macro_rules! run_expectation_try {
        ($expr:expr) => {
            match $expr {
                Ok(value) => value,
                Err(error) => return_expectation_error!(error),
            }
        };
    }

    // Interrupts are checked before any live report prefix is started, so this
    // branch cannot leave a printed `<short ID>.` without a completed record.
    if check_interrupted() {
        return_expectation_error!("interrupted");
    }

    let mut verified_q_scope = run_expectation_try!(initial_visible_scope_for_expectation(
        context.runtime.root,
        context.runtime.tree_source,
        expectation,
        &mut context.caches.xpec_state,
        &mut context.caches.visible_tree_oid,
        context.runtime.tree_context.checked_tree_oid
            != context.runtime.tree_context.against_tree_oid,
        context.active_lazy_full_scope_reset_ids,
    ));
    // Prepare the metadata needed to render an errored expectation before
    // printing the live report prefix. After `<short ID>.` is visible, later
    // fallible steps can build an ERROR record without doing more fallible
    // tree inspection.
    let started_report_error_visible_tree_oid =
        run_expectation_try!(context.runtime.visible_tree_oid(
            &mut context.caches.visible_tree_oid,
            &expectation.agent,
            &verified_q_scope,
        ));
    let started_report_error_timestamp =
        run_expectation_try!(unix_timestamp().map(format_record_timestamp));
    // Start the live report entry before the evaluator turn so the first dot
    // is visible while the expectation is still being evaluated.
    let mut started_report = run_expectation_try!(start_live_expectation_report(
        context.runtime.root,
        context.live_report_output,
        expectation,
    ));
    // Every fallible evaluator step after the report prefix is written is
    // inside `run_started_expectation_interrogation`. Do not route those
    // failures through a cancel-only progress helper: this match converts any
    // post-prefix error into the same public ERROR block shape as a normal
    // result.
    let completed_interrogation =
        match run_started_expectation_interrogation(context, expectation, &mut verified_q_scope) {
            Ok(completed) => completed,
            Err(error) => {
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
    let record = completed_interrogation.record;
    if let Some(report) = started_report.take() {
        // Live output is best-effort after the prefix; completion does not
        // create a return path that can drop the completed CheckRecord.
        report.finish_public_output_or_keep_state_report(&record);
    } else {
        run_expectation_try!(write_result_output_without_started_report(
            context.result_output,
            &record
        ));
    }
    // From this point on, the public per-expectation result has already been
    // written. Later state/cache/logging errors can fail the command, but they
    // occur after the visible result block.
    run_expectation_try!(record_finished_expectation(context, expectation, &record));
    let human_review_required = record.requires_human_review();
    let run_stop_signal_hit = completed_interrogation.break_after_tokens_hit
        || completed_interrogation.context_compaction_hit
        || completed_interrogation.stop_after_current_expectation;
    // Default check order stops after the first evaluated non-pass. Evaluator
    // stop signals are resource/control limits, so they stop after the current
    // result even when the result itself passed.
    let stop_run = run_stop_signal_hit
        || (!context.options.keep_going && (!record.passed() || human_review_required));
    if run_stop_signal_hit {
        context.interrogation_run_state.clear_thread_sessions();
    }
    Ok(ExpectationRunOutcome { record, stop_run })
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
}

fn run_started_expectation_interrogation<R: EvaluatorRunner>(
    context: &mut ExpectationRunContext<'_, '_, '_, R>,
    expectation: &SelectedExpectation,
    verified_q_scope: &mut Vec<String>,
) -> Result<CompletedInterrogation, String> {
    // `run_expectation` catches every error from this helper and finishes the
    // already-started report entry with an ERROR record before returning.
    let mut interrogation = interrogate_with_full_scope_retry(
        ScopedInterrogation {
            runtime: context.runtime,
            expectation,
            enforced_scope: verified_q_scope,
        },
        context.runner,
        context.diagnostic_log,
        context.interrogation_run_state,
        &mut context.caches.xpec_state,
        &mut context.caches.visible_tree_oid,
        context.options.break_after_tokens,
    )?;
    let mut break_after_tokens_hit =
        turn_exceeds_break_after_tokens(&interrogation, context.options.break_after_tokens);
    let mut context_compaction_hit = turn_has_context_compaction(&interrogation);
    let mut stop_after_current_expectation = interrogation.stop_after_current_expectation;

    let record_scope = interrogation.record.scope.clone();
    if !scope_is_within(&record_scope, verified_q_scope) {
        *verified_q_scope = record_scope.clone();
    }
    debug_assert!(scope_is_within(&record_scope, verified_q_scope));
    let proposed_q_scope = if interrogation.record.requires_human_review() {
        None
    } else {
        question_scope_suggestion_scope_for_independent_verification(
            context.runtime,
            &expectation.agent,
            interrogation.record.question_scope_suggestion.as_deref(),
            verified_q_scope,
            &mut context.caches.visible_tree_oid,
        )?
    };
    if let Some(proposed_scope) = proposed_q_scope {
        let mut verification_scope = proposed_scope.clone();
        let narrowed = interrogate_with_full_scope_retry(
            ScopedInterrogation {
                runtime: context.runtime,
                expectation,
                enforced_scope: &mut verification_scope,
            },
            context.runner,
            context.diagnostic_log,
            context.interrogation_run_state,
            &mut context.caches.xpec_state,
            &mut context.caches.visible_tree_oid,
            context.options.break_after_tokens,
        )?;
        break_after_tokens_hit |=
            turn_exceeds_break_after_tokens(&narrowed, context.options.break_after_tokens);
        context_compaction_hit |= turn_has_context_compaction(&narrowed);
        stop_after_current_expectation |= narrowed.stop_after_current_expectation;
        let accepted = narrowed_scope_is_accepted(&narrowed.record);
        write_scope_narrowing_event(
            context.diagnostic_log,
            &expectation.id,
            verified_q_scope,
            &proposed_scope,
            accepted,
        )?;
        if accepted {
            interrogation = narrowed;
        } else {
            interrogation.record.question_scope_suggestion = None;
            debug_assert_eq!(
                interrogation.record.scope.as_slice(),
                verified_q_scope.as_slice()
            );
        }
    }
    Ok(CompletedInterrogation {
        record: interrogation.record,
        break_after_tokens_hit,
        context_compaction_hit,
        stop_after_current_expectation,
    })
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
    context.caches.xpec_state.write_last_result_for_record(
        context.runtime.root,
        &context.runtime.tree_context.checked_tree_oid,
        expectation,
        &record,
    )?;
    write_expectation_result_event(context.diagnostic_log, &record)?;
    context.interrogation_run_state.clear_thread_sessions();
    Ok(ExpectationRunOutcome {
        record,
        stop_run: !context.options.keep_going,
    })
}
