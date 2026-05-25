use crate::check_cache::{
    cached_result_for_expectation, write_cache_hit, CachedResultLookup, CheckCacheHit,
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
use crate::check_selection::order_expectations_by_latest_non_pass;
use crate::check_types::{
    check_run_error, CachedExpectation, CheckOptions, CheckRecord, CheckRunError, CheckRunReport,
    NarrowingStats, SelectedExpectation,
};
use crate::config_types::AgentConfig;
#[cfg(test)]
use crate::config_types::CheckConfig;
use crate::evaluator_types::EvaluatorRunner;
use crate::hash::full_scope;
use crate::history::HistoryCache;
use crate::history_append::append_history_record_with_cache;
use crate::history_reuse::{is_reusable_history_record, latest_history_scope_with_cache};
use crate::logging::DiagnosticLogWriter;
use crate::scope::{is_strict_scope_subset, scope_is_within};
use crate::time::{parse_record_timestamp, unix_timestamp};
use crate::visible_tree_oid::VisibleTreeOidCache;
use std::cmp::Reverse;
use std::io::Write;
use std::path::Path;

pub(crate) struct CheckRunCaches {
    pub(crate) history: HistoryCache,
    pub(crate) visible_tree_oid: VisibleTreeOidCache,
}

impl CheckRunCaches {
    pub(crate) fn new() -> CheckRunCaches {
        CheckRunCaches {
            history: HistoryCache::new(),
            visible_tree_oid: VisibleTreeOidCache::new(),
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
    let mut cached = Vec::new();
    let total_expectations = runtime.config.expectations.len();
    let mut selected = 0usize;
    let silent = 0usize;
    let mut evaluated = 0usize;
    let mut narrowing = NarrowingStats::default();
    let non_selected = options.non_selected.clone();
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
                    cached.clone(),
                    CheckRunReportCounts {
                        evaluated,
                        selected,
                        skipped: skipped_count(total_expectations, records.len()),
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
    let check_work_queue = if !options.selectors_provided && !options.check_all {
        let selection = run_try!(default_check_selection(
            root,
            &config.agent,
            options,
            &mut caches.history,
            &mut caches.visible_tree_oid,
            run_try!(unix_timestamp()),
            &mut diagnostic_log,
        ));
        for CachedSelectionHit { expectation, hit } in selection.cached {
            let record = hit.record;
            if !record.passed() {
                run_try!(write_and_flush_result_output(&mut result_output, &record));
                records.push(record.clone());
            }
            cached.push(CachedExpectation {
                expectation,
                record,
            });
        }
        if selection.cached_failure_seen {
            let skipped = skipped_count(total_expectations, records.len());
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
        run_try!(order_expectations_by_latest_non_pass(
            root,
            selection.selected,
            &mut caches.history
        ))
    } else {
        selected = options.selected.len();
        run_try!(order_expectations_by_latest_non_pass(
            root,
            options.selected.clone(),
            &mut caches.history
        ))
    };
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

        // Cache selection can bypass reusable answer records, but it does not
        // erase the interrogation-policy scope seed.
        let mut enforced_scope = run_expectation_try!(latest_history_scope_with_cache(
            root,
            &config.agent,
            expectation,
            &mut caches.history
        ))
        .unwrap_or_else(full_scope);
        if !run_expectation_try!(caches
            .visible_tree_oid
            .missing_staged_scope_paths(root, &enforced_scope))
        .is_empty()
        {
            enforced_scope = full_scope();
        }
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
            &mut caches.visible_tree_oid,
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
                &mut caches.visible_tree_oid,
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
                let enforced_visible_tree_oid = run_expectation_try!(caches
                    .visible_tree_oid
                    .staged_visible_tree_oid(root, &config.agent, &enforced_scope));
                // A rejected narrowing invalidates only the evaluator's
                // proposed reusable cache scope. The original answer/evidence
                // came from the wider enforced scope, so keep that wide
                // interrogation result and restore its wide cache identity
                // instead of keeping anything from the narrowed verification
                // turn.
                interrogation.record = restore_record_to_enforced_scope(
                    initial_record,
                    &enforced_scope,
                    enforced_visible_tree_oid,
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
        let should_stop = run_stop_signal_hit && !options.check_all;
        if run_stop_signal_hit {
            interrogation_state.clear_thread_sessions();
        }
        run_expectation_try!(write_and_flush_result_output(
            &mut result_output,
            &interrogation.record
        ));
        records.push(interrogation.record);
        if should_stop {
            let skipped = skipped_count(total_expectations, records.len());
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
    let skipped = skipped_count(total_expectations, records.len());
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

struct CachedSelection {
    selected: Vec<SelectedExpectation>,
    cached: Vec<CachedSelectionHit>,
    cached_failure_seen: bool,
}

struct CachedSelectionHit {
    expectation: SelectedExpectation,
    hit: CheckCacheHit,
}

fn default_check_selection(
    root: &Path,
    agent: &AgentConfig,
    options: &CheckOptions,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    now: u64,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
) -> Result<CachedSelection, String> {
    let mut selected = Vec::new();
    let mut cached = Vec::new();
    let mut cached_failure_seen = false;
    for expectation in options.selected.clone() {
        match cached_result_for_expectation(
            root,
            agent,
            &expectation,
            history_cache,
            visible_tree_oid_cache,
            CachedResultLookup {
                now,
                include_same_tree: !options.ignore_cache,
                include_cooldown: !options.ignore_cooldown,
            },
        )? {
            Some(hit) => {
                cached_failure_seen |= !hit.record.passed();
                if let Some(writer) = diagnostic_log.as_deref_mut() {
                    write_cache_hit(writer, &hit)?;
                }
                cached.push(CachedSelectionHit { expectation, hit });
            }
            None => selected.push(expectation),
        }
    }
    if cached_failure_seen {
        selected.clear();
        cached.sort_by_key(|hit| Reverse(record_timestamp_sort_key(&hit.hit.record)));
    }
    Ok(CachedSelection {
        selected,
        cached,
        cached_failure_seen,
    })
}

fn record_timestamp_sort_key(record: &CheckRecord) -> u64 {
    parse_record_timestamp(&record.timestamp).unwrap_or(0)
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

fn skipped_count(total_expectations: usize, output_records: usize) -> usize {
    total_expectations.saturating_sub(output_records)
}
