use crate::check_cache::{
    cached_failure_for_expectation, cached_record_for_expectation,
    check_work_queue_after_current_pass_cache, write_cache_hit,
};
use crate::check_interrogation_policy::{
    interrogate_with_full_scope_retry, narrowed_scope_is_accepted,
    restore_record_to_enforced_scope, turn_exceeds_break_after_tokens, turn_has_context_compaction,
    write_scope_narrowing_event, ScopedInterrogation,
};
use crate::check_interrogation_state::{CheckRuntime, InterrogationState};
use crate::check_order_state::{
    write_latest_non_pass_error_with_cache, write_latest_non_pass_record_with_cache,
};
use crate::check_output::{record_requires_human_review, write_and_flush_result_output};
use crate::check_preflight::check_interrupted;
use crate::check_selection::{
    cooldown_filtered_check_work_queue, order_expectations_by_latest_non_pass, CheckWorkQueue,
};
use crate::check_types::{
    check_run_error, CheckOptions, CheckRecord, CheckRunError, CheckRunReport, NarrowingStats,
    SelectedExpectation,
};
#[cfg(test)]
use crate::config_types::CheckConfig;
use crate::evaluator_types::EvaluatorRunner;
use crate::hash::full_scope;
use crate::history::HistoryCache;
use crate::history_append::append_history_record_with_cache;
use crate::history_reuse::{is_reusable_history_record, latest_history_scope_with_cache};
use crate::logging::DiagnosticLogWriter;
use crate::scope::{is_strict_scope_subset, scope_is_within};
use crate::scope_hash::ScopeHashCache;
use crate::time::unix_timestamp;
use std::io::Write;
#[cfg(test)]
use std::path::Path;

pub(crate) struct CheckRunCaches {
    pub(crate) history: HistoryCache,
    pub(crate) scope_hash: ScopeHashCache,
}

impl CheckRunCaches {
    pub(crate) fn new() -> CheckRunCaches {
        CheckRunCaches {
            history: HistoryCache::new(),
            scope_hash: ScopeHashCache::new(),
        }
    }
}

#[cfg(test)]
pub(crate) fn run_check_with_runner<R: EvaluatorRunner>(
    root: &Path,
    snapshot_root: &Path,
    config: &CheckConfig,
    options: &CheckOptions,
    runner: &mut R,
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    result_output: Option<&mut dyn Write>,
) -> Result<CheckRunReport, CheckRunError> {
    let mut caches = CheckRunCaches::new();
    let runtime = CheckRuntime {
        root,
        snapshot_root,
        config,
    };
    run_check_with_runner_and_caches(
        runtime,
        options,
        runner,
        diagnostic_log,
        result_output,
        &mut caches,
    )
}

pub(crate) fn run_check_with_runner_and_caches<R: EvaluatorRunner>(
    runtime: CheckRuntime<'_>,
    options: &CheckOptions,
    runner: &mut R,
    mut diagnostic_log: Option<&mut DiagnosticLogWriter>,
    mut result_output: Option<&mut dyn Write>,
    caches: &mut CheckRunCaches,
) -> Result<CheckRunReport, CheckRunError> {
    let mut records = Vec::new();
    // Public selected/skipped counts describe command selection. Check-only
    // work-saving filters are tracked separately as `silent` so the invariant
    // `selected + skipped == all expectations` remains stable.
    let selected = options.selected.len();
    let skipped = options.skipped;
    let mut silent = 0usize;
    let mut evaluated = 0usize;
    let mut narrowing = NarrowingStats::default();
    let mut non_selected = options.non_selected.clone();
    let root = runtime.root;
    let config = runtime.config;
    // Per-run state is shared so equal canonical enforced scopes can reuse one
    // ephemeral evaluator thread; InterrogationState stores thread IDs by scope,
    // so different enforced scopes still start separate threads within the run.
    let mut interrogation_state = InterrogationState::new();
    macro_rules! current_error {
        ($error:expr) => {
            check_run_error(
                $error,
                check_run_report(
                    records.clone(),
                    non_selected.clone(),
                    evaluated,
                    selected,
                    skipped,
                    silent,
                    narrowing,
                ),
            )
        };
    }
    macro_rules! run_try {
        ($expr:expr) => {
            $expr.map_err(|err| current_error!(err.to_string()))?
        };
    }
    let cooldown_queue = if options.ignore_cooldown || options.check_all {
        CheckWorkQueue {
            selected: options.selected.clone(),
            skipped: Vec::new(),
        }
    } else {
        match cooldown_filtered_check_work_queue(
            root,
            &config.agent,
            options.selected.clone(),
            &mut caches.history,
            run_try!(unix_timestamp()),
        ) {
            Ok(cooldown_queue) => cooldown_queue,
            Err(err) => {
                mark_expectations_silent(err.skipped, &mut non_selected, &mut silent);
                return Err(current_error!(err.error));
            }
        }
    };
    let CheckWorkQueue {
        selected: cooldown_selected,
        skipped: cooldown_skipped,
    } = cooldown_queue;
    mark_expectations_silent(cooldown_skipped, &mut non_selected, &mut silent);
    let ordered_cooldown_selected = run_try!(order_expectations_by_latest_non_pass(
        root,
        cooldown_selected,
        &mut caches.history
    ));
    let check_work_queue = if options.ignore_cache || options.check_all {
        ordered_cooldown_selected
    } else {
        // Passing exact-cache hits satisfy candidates before the
        // selected-expectation work loop below. They remain command-selected
        // expectations, but contribute to silent because no evaluator turn or
        // per-expectation stdout is needed. Ordering happens before this skip
        // so every selected expectation participates in the
        // latest-non-pass ordering policy. Failed exact-cache hits stay in the
        // work queue and are reported below.
        let cache_selection = run_try!(check_work_queue_after_current_pass_cache(
            root,
            &config.agent,
            ordered_cooldown_selected,
            &mut caches.history,
            &mut caches.scope_hash,
        ));
        for (expectation, hit) in cache_selection.skipped_passes {
            mark_expectations_silent(vec![expectation], &mut non_selected, &mut silent);
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                run_try!(write_cache_hit(writer, &hit));
            }
        }
        cache_selection.selected
    };
    let stop_after_non_pass = !options.check_all;
    for expectation in &check_work_queue {
        macro_rules! return_expectation_error {
            ($error:expr) => {{
                let error = $error.to_string();
                if let Err(marker_error) =
                    write_latest_non_pass_error_with_cache(root, expectation, &mut caches.history)
                {
                    return Err(current_error!(format!(
                        "{}; failed to record latest non-pass error: {}",
                        error, marker_error
                    )));
                }
                return Err(current_error!(error));
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
        // Each branch that produces a selected CheckRecord writes and flushes
        // that record before moving to the next expectation. Silent passing
        // cache hits have already been removed from `check_work_queue` unless
        // `--all` requested a visible result for every selected expectation.
        if check_interrupted() {
            return_expectation_error!("interrupted");
        }
        if !options.ignore_cache {
            let cache_hit = if options.check_all {
                cached_record_for_expectation(
                    root,
                    &config.agent,
                    expectation,
                    &mut caches.history,
                    &mut caches.scope_hash,
                )
            } else {
                cached_failure_for_expectation(
                    root,
                    &config.agent,
                    expectation,
                    &mut caches.history,
                    &mut caches.scope_hash,
                )
            };
            if let Some(hit) = run_expectation_try!(cache_hit) {
                let should_stop = stop_after_non_pass && !hit.record.passed();
                if let Some(writer) = diagnostic_log.as_deref_mut() {
                    run_expectation_try!(write_cache_hit(writer, &hit));
                }
                run_expectation_try!(write_and_flush_result_output(
                    &mut result_output,
                    &hit.record
                ));
                records.push(hit.record);
                if should_stop {
                    return Ok(check_run_report(
                        records,
                        non_selected,
                        evaluated,
                        selected,
                        skipped,
                        silent,
                        narrowing,
                    ));
                }
                continue;
            }
        }

        // `--ignore-cache` bypasses reusable answer records in the branch above,
        // but it does not erase the interrogation-policy scope seed.
        let mut enforced_scope = run_expectation_try!(latest_history_scope_with_cache(
            root,
            &config.agent,
            expectation,
            &mut caches.history
        ))
        .unwrap_or_else(full_scope);
        // Response-format problems and evaluator runner/model failures are
        // handled inside this call: they become non-pass review records that
        // are written through `write_latest_non_pass_record_with_cache` below.
        // Err here means infrastructure failed before such a record could be
        // produced.
        let mut interrogation = run_expectation_try!(interrogate_with_full_scope_retry(
            ScopedInterrogation {
                root,
                runtime: &runtime,
                expectation,
                enforced_scope: &mut enforced_scope,
            },
            runner,
            &mut diagnostic_log,
            &mut interrogation_state,
            &mut caches.scope_hash,
            options.break_after_tokens,
        ));
        evaluated += 1;
        let mut break_after_tokens_hit =
            turn_exceeds_break_after_tokens(&interrogation, options.break_after_tokens);
        let mut context_compaction_hit = turn_has_context_compaction(&interrogation);
        let mut stop_after_current_expectation = interrogation.stop_after_current_expectation;

        let record_scope = interrogation.record.scope.clone();
        // Interrogation finalization rejects evaluator-proposed widening before
        // this point. Restricted widening becomes an enforced-scope `idk` so
        // full-scope retry can decide whether the restricted context was
        // insufficient; full-scope malformed/unparseable states remain review
        // records.
        debug_assert!(scope_is_within(&record_scope, &enforced_scope));
        // Cache-spec narrowing verification applies only to verified answers.
        // Non-answer states (`idk`, `malformed`, unparseable) are never reusable
        // cache records and are handled by the review-required/idk policy above.
        // Token-break and context-compaction signals are run-level stop
        // signals in default mode; they do not skip the independent
        // verification needed to trust a strictly narrower cache scope for
        // this expectation's final record.
        if !record_requires_human_review(&interrogation.record)
            && is_strict_scope_subset(&record_scope, &enforced_scope)
        {
            narrowing.attempted += 1;
            // A narrower scope from one evaluator response becomes reusable
            // only when an independent interrogation returns either the same
            // answer or an incorrect answer.
            let initial_record = interrogation.record.clone();
            let mut verification_scope = record_scope.clone();
            let narrowed = run_expectation_try!(interrogate_with_full_scope_retry(
                ScopedInterrogation {
                    root,
                    runtime: &runtime,
                    expectation,
                    enforced_scope: &mut verification_scope,
                },
                runner,
                &mut diagnostic_log,
                &mut interrogation_state,
                &mut caches.scope_hash,
                options.break_after_tokens,
            ));
            break_after_tokens_hit |=
                turn_exceeds_break_after_tokens(&narrowed, options.break_after_tokens);
            context_compaction_hit |= turn_has_context_compaction(&narrowed);
            stop_after_current_expectation |= narrowed.stop_after_current_expectation;
            let accepted = narrowed_scope_is_accepted(&interrogation.record, &narrowed.record);
            if accepted {
                narrowing.accepted += 1;
            } else {
                narrowing.rejected += 1;
            }
            run_expectation_try!(write_scope_narrowing_event(
                &mut diagnostic_log,
                &expectation.id,
                &enforced_scope,
                &record_scope,
                accepted,
                &initial_record,
                &narrowed.record,
            ));
            if accepted {
                interrogation = narrowed;
            } else {
                let enforced_scope_hash = run_expectation_try!(caches
                    .scope_hash
                    .staged_scope_hash(root, &config.agent, &enforced_scope));
                // A rejected narrowing invalidates only the evaluator's
                // proposed reusable cache scope. The original answer/evidence
                // came from the wider enforced scope, so keep that wide
                // interrogation result and restore its wide cache identity
                // instead of keeping anything from the narrowed verification
                // turn.
                interrogation.record = restore_record_to_enforced_scope(
                    initial_record,
                    &enforced_scope,
                    enforced_scope_hash,
                );
            }
        }
        // Correct and incorrect parsed answers are reusable for every
        // expectation shape, including free-form exact strings. Human-review
        // states such as idk, malformed, and unparseable responses are not
        // written to history.
        if is_reusable_history_record(&interrogation.record) {
            run_expectation_try!(append_history_record_with_cache(
                root,
                expectation,
                &interrogation.record,
                &mut caches.history,
            ));
        }
        run_expectation_try!(write_latest_non_pass_record_with_cache(
            root,
            expectation,
            &interrogation.record,
            &mut caches.history
        ));
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            run_expectation_try!(writer.write_record(&interrogation.record));
        }
        let run_stop_signal_hit =
            break_after_tokens_hit || context_compaction_hit || stop_after_current_expectation;
        let should_stop =
            stop_after_non_pass && (!interrogation.record.passed() || run_stop_signal_hit);
        if run_stop_signal_hit {
            interrogation_state.clear_thread_sessions();
        }
        run_expectation_try!(write_and_flush_result_output(
            &mut result_output,
            &interrogation.record
        ));
        records.push(interrogation.record);
        if should_stop {
            return Ok(check_run_report(
                records,
                non_selected,
                evaluated,
                selected,
                skipped,
                silent,
                narrowing,
            ));
        }
    }
    Ok(check_run_report(
        records,
        non_selected,
        evaluated,
        selected,
        skipped,
        silent,
        narrowing,
    ))
}

fn check_run_report(
    records: Vec<CheckRecord>,
    non_selected: Vec<SelectedExpectation>,
    evaluated: usize,
    selected: usize,
    skipped: usize,
    silent: usize,
    narrowing: NarrowingStats,
) -> CheckRunReport {
    CheckRunReport {
        records,
        non_selected,
        evaluated,
        selected,
        skipped,
        silent,
        narrowing,
    }
}

fn mark_expectations_silent(
    expectations: Vec<SelectedExpectation>,
    non_selected: &mut Vec<SelectedExpectation>,
    silent: &mut usize,
) {
    let count = expectations.len();
    non_selected.extend(expectations);
    *silent += count;
}
