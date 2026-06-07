use super::progress::cancel_progress_on_error;
use super::CheckRunCaches;
use crate::check::command::output::{
    record_requires_human_review, start_check_progress_output, write_and_flush_result_output,
    SharedCheckOutput,
};
use crate::check::core::types::{CheckOptions, CheckRecord, SelectedExpectation};
use crate::check::interrogation::policy::{
    interrogate_with_full_scope_retry, narrowed_scope_is_accepted,
    question_scope_suggestion_should_get_independent_verification, turn_exceeds_break_after_tokens,
    turn_has_context_compaction, write_scope_narrowing_event, ScopedInterrogation,
};
use crate::check::interrogation::state::{
    initial_visible_scope_for_expectation, CheckRuntime, InterrogationRunState,
};
use crate::check::run::order_state::{
    write_latest_non_pass_error_with_cache, write_latest_non_pass_record_with_cache,
};
use crate::evaluator::EvaluatorRunner;
use crate::history::{append_current_history_record_with_cache, is_reusable_history_record};
use crate::logs::{DiagnosticLogWriter, DiagnosticRecordEvent};
use crate::platform::check_interrupted;
use crate::scope::{sanitize_scope, scope_is_within};
use std::collections::BTreeSet;
use std::io::Write;
use std::time::Instant;

pub(super) struct ExpectationRunContext<'a, 'out, 'log, R: EvaluatorRunner> {
    pub(super) runtime: &'a CheckRuntime<'a>,
    pub(super) options: &'a CheckOptions,
    pub(super) active_lazy_full_scope_reset_ids: &'a BTreeSet<String>,
    pub(super) runner: &'a mut R,
    pub(super) diagnostic_log: &'a mut Option<&'log mut DiagnosticLogWriter>,
    pub(super) result_output: &'a mut Option<&'out mut dyn Write>,
    pub(super) progress_output: &'a Option<SharedCheckOutput>,
    pub(super) started: Instant,
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
    macro_rules! return_expectation_error {
        ($error:expr) => {{
            let error = $error.to_string();
            if let Err(marker_error) = write_latest_non_pass_error_with_cache(
                context.runtime.root,
                expectation,
                &mut context.caches.history,
            ) {
                return Err(format!(
                    "{}; failed to record latest non-pass error: {}",
                    error, marker_error
                ));
            }
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

    if check_interrupted() {
        return_expectation_error!("interrupted");
    }

    let mut verified_q_scope = run_expectation_try!(initial_visible_scope_for_expectation(
        context.runtime.root,
        context.runtime.tree_source,
        expectation,
        &mut context.caches.history,
        &mut context.caches.visible_tree_oid,
        context.active_lazy_full_scope_reset_ids,
    ));
    let mut progress = match context.progress_output.as_ref() {
        Some(output) => Some(run_expectation_try!(start_check_progress_output(
            output.clone(),
            &expectation.display_id,
        ))),
        None => None,
    };
    let mut interrogation = run_expectation_try!(cancel_progress_on_error(
        interrogate_with_full_scope_retry(
            ScopedInterrogation {
                runtime: context.runtime,
                expectation,
                enforced_scope: &mut verified_q_scope,
            },
            context.runner,
            context.diagnostic_log,
            context.interrogation_run_state,
            &mut context.caches.history,
            &mut context.caches.visible_tree_oid,
            context.options.break_after_tokens,
        ),
        &mut progress,
    ));
    let mut break_after_tokens_hit =
        turn_exceeds_break_after_tokens(&interrogation, context.options.break_after_tokens);
    let mut context_compaction_hit = turn_has_context_compaction(&interrogation);
    let mut stop_after_current_expectation = interrogation.stop_after_current_expectation;

    let record_scope = interrogation.record.scope.clone();
    debug_assert!(scope_is_within(&record_scope, &verified_q_scope));
    if !record_requires_human_review(&interrogation.record)
        && run_expectation_try!(cancel_progress_on_error(
            question_scope_suggestion_should_get_independent_verification(
                context.runtime,
                &expectation.agent,
                interrogation.record.question_scope_suggestion.as_deref(),
                &verified_q_scope,
                &mut context.caches.visible_tree_oid,
            ),
            &mut progress,
        ))
    {
        let initial_record = interrogation.record.clone();
        let proposed_scope = run_expectation_try!(cancel_progress_on_error(
            sanitize_scope(
                initial_record
                    .question_scope_suggestion
                    .as_deref()
                    .expect("suggestion passed the file-count verification gate"),
            ),
            &mut progress,
        ));
        let mut verification_scope = proposed_scope.clone();
        let narrowed = run_expectation_try!(cancel_progress_on_error(
            interrogate_with_full_scope_retry(
                ScopedInterrogation {
                    runtime: context.runtime,
                    expectation,
                    enforced_scope: &mut verification_scope,
                },
                context.runner,
                context.diagnostic_log,
                context.interrogation_run_state,
                &mut context.caches.history,
                &mut context.caches.visible_tree_oid,
                context.options.break_after_tokens,
            ),
            &mut progress,
        ));
        break_after_tokens_hit |=
            turn_exceeds_break_after_tokens(&narrowed, context.options.break_after_tokens);
        context_compaction_hit |= turn_has_context_compaction(&narrowed);
        stop_after_current_expectation |= narrowed.stop_after_current_expectation;
        let accepted = narrowed_scope_is_accepted(&narrowed.record, &proposed_scope);
        run_expectation_try!(cancel_progress_on_error(
            write_scope_narrowing_event(
                context.diagnostic_log,
                &expectation.id,
                &verified_q_scope,
                &proposed_scope,
                accepted,
                &initial_record,
                &narrowed.record,
            ),
            &mut progress,
        ));
        if accepted {
            interrogation = narrowed;
        } else {
            interrogation.record.question_scope_suggestion = None;
            debug_assert_eq!(interrogation.record.scope, verified_q_scope);
        }
    }
    if let Some(progress) = progress.take() {
        run_expectation_try!(progress.finish_with_record(&interrogation.record));
    } else {
        run_expectation_try!(write_and_flush_result_output(
            context.result_output,
            &interrogation.record,
            context.started.elapsed()
        ));
    }
    if is_reusable_history_record(&interrogation.record) {
        run_expectation_try!(append_current_history_record_with_cache(
            context.runtime.root,
            context.runtime.tree_source,
            expectation,
            &interrogation.record,
            &mut context.caches.history,
            &mut context.caches.visible_tree_oid,
        ));
    }
    run_expectation_try!(write_latest_non_pass_record_with_cache(
        context.runtime.root,
        expectation,
        &interrogation.record,
        &mut context.caches.history
    ));
    if let Some(writer) = context.diagnostic_log.as_deref_mut() {
        run_expectation_try!(
            writer.write_record_event(DiagnosticRecordEvent::Expectation, &interrogation.record)
        );
    }
    let run_stop_signal_hit =
        break_after_tokens_hit || context_compaction_hit || stop_after_current_expectation;
    let stop_run =
        !context.options.keep_going && (!interrogation.record.passed() || run_stop_signal_hit);
    if run_stop_signal_hit {
        context.interrogation_run_state.clear_thread_sessions();
    }
    Ok(ExpectationRunOutcome {
        record: interrogation.record,
        stop_run,
    })
}
