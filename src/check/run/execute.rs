use crate::check::command::output::{record_requires_human_review, write_and_flush_result_output};
use crate::check::core::types::{
    check_run_error, CachedExpectation, CheckOptions, CheckRecord, CheckRunError, CheckRunReport,
    NarrowingStats, SelectedExpectation,
};
use crate::check::interrogation::policy::{
    interrogate_with_full_scope_retry, narrowed_scope_is_accepted,
    q_scope_suggestion_should_get_independent_verification, turn_exceeds_break_after_tokens,
    turn_has_context_compaction, write_scope_narrowing_event, ScopedInterrogation,
};
use crate::check::interrogation::state::{
    initial_visible_scope_for_expectation, CheckRuntime, InterrogationRunState,
};
use crate::check::run::lazy_reset::clear_active_lazy_full_scope_reset;
use crate::check::run::order_state::{
    write_latest_non_pass_error_with_cache, write_latest_non_pass_record_with_cache,
};
use crate::check::run::selection::{
    order_expectations_by_latest_non_pass, select_expectations_after_cache, CachedFailureMode,
    CachedSelectionContext, CachedSelectionHit,
};
#[cfg(test)]
use crate::config_types::CheckConfig;
use crate::evaluator::EvaluatorRunner;
use crate::git::VisibleTreeOidCache;
use crate::history::{
    append_current_history_record_with_cache, is_reusable_history_record, HistoryCache,
};
use crate::logs::{DiagnosticLogWriter, DiagnosticRecordEvent};
use crate::platform::check_interrupted;
use crate::scope::{sanitize_scope, scope_is_within};
use crate::time::unix_timestamp;
use std::collections::BTreeSet;
use std::io::Write;
#[cfg(test)]
use std::path::Path;
use std::time::Instant;

pub(crate) struct CheckRunCaches {
    pub(crate) history: HistoryCache,
    pub(crate) visible_tree_oid: VisibleTreeOidCache,
}

impl CheckRunCaches {
    pub(crate) fn new() -> CheckRunCaches {
        CheckRunCaches {
            history: HistoryCache::default(),
            visible_tree_oid: VisibleTreeOidCache::new(),
        }
    }
}

pub(crate) struct CheckRunSideEffects<'out, 'cache, 'log> {
    pub(crate) diagnostic_log: Option<&'log mut DiagnosticLogWriter>,
    pub(crate) result_output: Option<&'out mut dyn Write>,
    pub(crate) started: Instant,
    pub(crate) caches: &'cache mut CheckRunCaches,
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
    let runtime = CheckRuntime::fixed(root, snapshot_root, config);
    let active_lazy_full_scope_reset_ids = BTreeSet::new();
    run_check_with_runner_and_caches(
        runtime,
        options,
        &active_lazy_full_scope_reset_ids,
        runner,
        CheckRunSideEffects {
            diagnostic_log,
            result_output,
            started: Instant::now(),
            caches: &mut caches,
        },
    )
}

pub(crate) fn run_check_with_runner_and_caches<R: EvaluatorRunner>(
    runtime: CheckRuntime<'_>,
    options: &CheckOptions,
    active_lazy_full_scope_reset_ids: &BTreeSet<String>,
    runner: &mut R,
    side_effects: CheckRunSideEffects<'_, '_, '_>,
) -> Result<CheckRunReport, CheckRunError> {
    let CheckRunSideEffects {
        mut diagnostic_log,
        mut result_output,
        started,
        caches,
    } = side_effects;
    let mut records = Vec::new();
    let mut cached = Vec::new();
    let total_expectations = runtime.config.expectations.len();
    let mut selected = 0usize;
    let silent = 0usize;
    let mut evaluated = 0usize;
    let mut narrowing = NarrowingStats::default();
    let non_selected = options.non_selected.clone();
    let root = runtime.root;
    macro_rules! current_error {
        ($error:expr) => {
            check_run_error(
                $error,
                check_run_report(
                    records.clone(),
                    non_selected.clone(),
                    cached.clone(),
                    CheckRunReportCounts {
                        evaluated,
                        selected,
                        skipped: skipped_count(total_expectations, &records, &cached),
                        silent,
                    },
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
    // This is shared run state, not a shared evaluator thread. Evaluator
    // threads are stored in a pool keyed by model plus visibleTreeOid (and
    // stricter instruction inputs), so full-scope retries and narrowed-scope
    // verifications start different app-server sessions when they see
    // different visible trees.
    let mut interrogation_run_state = run_try!(InterrogationRunState::new(runtime.no_sandbox()));
    let selection = run_try!(select_expectations_after_cache(
        CachedSelectionContext {
            root,
            source: runtime.tree_source,
            history_cache: &mut caches.history,
            visible_tree_oid_cache: &mut caches.visible_tree_oid,
            active_lazy_full_scope_reset_ids,
            diagnostic_log: &mut diagnostic_log,
        },
        options,
        run_try!(unix_timestamp()),
        if options.selectors_provided {
            CachedFailureMode::Continue
        } else {
            CachedFailureMode::StopDefaultSelection
        },
    ));
    for CachedSelectionHit { expectation, hit } in selection.cached {
        let record = hit.record;
        if !record.passed() {
            run_try!(write_and_flush_result_output(
                &mut result_output,
                &record,
                started.elapsed()
            ));
            records.push(record.clone());
        }
        cached.push(CachedExpectation {
            expectation,
            record,
        });
    }
    if selection.cached_failure_seen && selection.selected.is_empty() && !options.selectors_provided
    {
        let skipped = skipped_count(total_expectations, &records, &cached);
        return Ok(check_run_report(
            records,
            non_selected,
            cached,
            CheckRunReportCounts {
                evaluated,
                selected,
                skipped,
                silent,
            },
            narrowing,
        ));
    }
    selected = selection.selected.len();
    let check_work_queue = run_try!(order_expectations_by_latest_non_pass(
        root,
        selection.selected,
        &mut caches.history
    ));
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
        // Each selected CheckRecord is written and flushed before moving to
        // the next expectation.
        if check_interrupted() {
            return_expectation_error!("interrupted");
        }

        let active_lazy_full_scope_reset =
            active_lazy_full_scope_reset_ids.contains(&expectation.id);
        let mut verified_q_scope = run_expectation_try!(initial_visible_scope_for_expectation(
            root,
            runtime.tree_source,
            expectation,
            &mut caches.history,
            &mut caches.visible_tree_oid,
            active_lazy_full_scope_reset_ids,
        ));
        // Response-format problems and evaluator runner/model failures are
        // handled inside this call: they become non-pass review records that
        // are written through `write_latest_non_pass_record_with_cache` below.
        // Err here means infrastructure failed before such a record could be
        // produced.
        let mut interrogation = run_expectation_try!(interrogate_with_full_scope_retry(
            ScopedInterrogation {
                runtime: &runtime,
                expectation,
                enforced_scope: &mut verified_q_scope,
            },
            runner,
            &mut diagnostic_log,
            &mut interrogation_run_state,
            &mut caches.visible_tree_oid,
            options.break_after_tokens,
        ));
        evaluated += 1;
        let mut break_after_tokens_hit =
            turn_exceeds_break_after_tokens(&interrogation, options.break_after_tokens);
        let mut context_compaction_hit = turn_has_context_compaction(&interrogation);
        let mut stop_after_current_expectation = interrogation.stop_after_current_expectation;

        let record_scope = interrogation.record.scope.clone();
        // Interrogation finalization records the enforced scope before this
        // point. A restricted insufficient-evidence error has already had its
        // full-scope retry; final errors remain review records.
        debug_assert!(scope_is_within(&record_scope, &verified_q_scope));
        // Cache-spec narrowing verification applies only to verified answers.
        // Error and unparsable states are never reusable cache records and are
        // handled by the review-required policy above.
        // Token-break and context-compaction signals are run-level stop
        // signals in default mode; they do not skip the independent
        // verification needed to trust a strictly narrower cache scope for
        // this expectation's final record.
        if !record_requires_human_review(&interrogation.record)
            && run_expectation_try!(q_scope_suggestion_should_get_independent_verification(
                &runtime,
                &expectation.agent,
                interrogation.record.suggested_q_scope.as_deref(),
                &verified_q_scope,
                &mut caches.visible_tree_oid,
            ))
        {
            narrowing.attempted += 1;
            // A q-scope suggestion becomes reusable only when an independent
            // interrogation under that suggested scope returns a valid answer.
            let initial_record = interrogation.record.clone();
            let proposed_scope = run_expectation_try!(sanitize_scope(
                initial_record
                    .suggested_q_scope
                    .as_deref()
                    .expect("suggestion passed the file-count verification gate"),
                &expectation.agent,
            ));
            let mut verification_scope = proposed_scope.clone();
            let narrowed = run_expectation_try!(interrogate_with_full_scope_retry(
                ScopedInterrogation {
                    runtime: &runtime,
                    expectation,
                    enforced_scope: &mut verification_scope,
                },
                runner,
                &mut diagnostic_log,
                &mut interrogation_run_state,
                &mut caches.visible_tree_oid,
                options.break_after_tokens,
            ));
            break_after_tokens_hit |=
                turn_exceeds_break_after_tokens(&narrowed, options.break_after_tokens);
            context_compaction_hit |= turn_has_context_compaction(&narrowed);
            stop_after_current_expectation |= narrowed.stop_after_current_expectation;
            let accepted = narrowed_scope_is_accepted(&narrowed.record, &proposed_scope);
            if accepted {
                narrowing.accepted += 1;
            } else {
                narrowing.rejected += 1;
            }
            run_expectation_try!(write_scope_narrowing_event(
                &mut diagnostic_log,
                &expectation.id,
                &verified_q_scope,
                &proposed_scope,
                accepted,
                &initial_record,
                &narrowed.record,
            ));
            if accepted {
                interrogation = narrowed;
            } else {
                // A rejected q-scope suggestion remains an evaluator-provided
                // claim, but it is not a verified q-scope available for final
                // output, answer history, or future visible-scope formation.
                // Keep the original wide interrogation record; the rejected
                // candidate stays in the diagnostic narrowing event only.
                interrogation.record.suggested_q_scope = None;
                debug_assert_eq!(interrogation.record.scope, verified_q_scope);
            }
        }
        // Correct and incorrect parsed answers are reusable for every
        // expectation shape, including free-form exact strings. Error and
        // unparsable responses are not written to history.
        if is_reusable_history_record(&interrogation.record) {
            run_expectation_try!(append_current_history_record_with_cache(
                root,
                runtime.tree_source,
                expectation,
                &interrogation.record,
                &mut caches.history,
                &mut caches.visible_tree_oid,
            ));
        }
        run_expectation_try!(write_latest_non_pass_record_with_cache(
            root,
            expectation,
            &interrogation.record,
            &mut caches.history
        ));
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            run_expectation_try!(writer
                .write_record_event(DiagnosticRecordEvent::Expectation, &interrogation.record));
        }
        let run_stop_signal_hit =
            break_after_tokens_hit || context_compaction_hit || stop_after_current_expectation;
        let should_stop =
            !options.keep_going && (!interrogation.record.passed() || run_stop_signal_hit);
        if run_stop_signal_hit {
            interrogation_run_state.clear_thread_sessions();
        }
        run_expectation_try!(write_and_flush_result_output(
            &mut result_output,
            &interrogation.record,
            started.elapsed()
        ));
        records.push(interrogation.record);
        if active_lazy_full_scope_reset {
            run_expectation_try!(clear_active_lazy_full_scope_reset(root, expectation));
        }
        if should_stop {
            let skipped = skipped_count(total_expectations, &records, &cached);
            return Ok(check_run_report(
                records,
                non_selected,
                cached,
                CheckRunReportCounts {
                    evaluated,
                    selected,
                    skipped,
                    silent,
                },
                narrowing,
            ));
        }
    }
    let skipped = skipped_count(total_expectations, &records, &cached);
    Ok(check_run_report(
        records,
        non_selected,
        cached,
        CheckRunReportCounts {
            evaluated,
            selected,
            skipped,
            silent,
        },
        narrowing,
    ))
}

fn check_run_report(
    records: Vec<CheckRecord>,
    non_selected: Vec<SelectedExpectation>,
    cached: Vec<CachedExpectation>,
    counts: CheckRunReportCounts,
    narrowing: NarrowingStats,
) -> CheckRunReport {
    CheckRunReport {
        records,
        non_selected,
        cached,
        evaluated: counts.evaluated,
        selected: counts.selected,
        skipped: counts.skipped,
        silent: counts.silent,
        narrowing,
    }
}

struct CheckRunReportCounts {
    evaluated: usize,
    selected: usize,
    skipped: usize,
    silent: usize,
}

fn skipped_count(
    total_expectations: usize,
    records: &[CheckRecord],
    cached: &[CachedExpectation],
) -> usize {
    total_expectations.saturating_sub(summary_result_count(records, cached))
}

fn summary_result_count(records: &[CheckRecord], cached: &[CachedExpectation]) -> usize {
    let mut seen = BTreeSet::new();
    let mut count = 0usize;
    for record in records {
        if seen.insert(record.id.clone()) {
            count += 1;
        }
    }
    for cached in cached {
        let id = if cached.record.id.is_empty() {
            &cached.expectation.id
        } else {
            &cached.record.id
        };
        if seen.insert(id.clone()) {
            count += 1;
        }
    }
    count
}
