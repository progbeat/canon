use super::expectation::{run_expectation, ExpectationRunContext};
use super::report::{check_run_report, skipped_count, CheckRunReportCounts};
use super::CheckRunSideEffects;
use crate::check::command::output::write_and_flush_result_output;
use crate::check::core::types::{
    check_run_error, CachedExpectation, CheckOptions, CheckRecord, CheckRunError, CheckRunReport,
    NarrowingStats,
};
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::check::run::selection::{
    order_expectations_by_latest_non_pass, select_expectations_after_cache, CachedFailureMode,
    CachedSelectionContext, CachedSelectionHit,
};
use crate::evaluator::EvaluatorRunner;
use crate::time::unix_timestamp;
use std::collections::BTreeSet;

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
        progress_output,
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
    write_cached_failures(
        selection.cached,
        &mut records,
        &mut cached,
        &mut result_output,
        started,
    )
    .map_err(|err| current_error!(err))?;
    if selection.cached_failure_seen && selection.selected.is_empty() && !options.selectors_provided
    {
        return Ok(current_report(
            records,
            non_selected,
            cached,
            total_expectations,
            evaluated,
            selected,
            silent,
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
        let outcome = match run_expectation(
            &mut ExpectationRunContext {
                runtime: &runtime,
                options,
                active_lazy_full_scope_reset_ids,
                runner,
                diagnostic_log: &mut diagnostic_log,
                result_output: &mut result_output,
                progress_output: &progress_output,
                started,
                caches,
                interrogation_run_state: &mut interrogation_run_state,
            },
            expectation,
        ) {
            Ok(outcome) => outcome,
            Err(error) => return Err(current_error!(error)),
        };
        evaluated += 1;
        narrowing.attempted += outcome.narrowing.attempted;
        narrowing.accepted += outcome.narrowing.accepted;
        narrowing.rejected += outcome.narrowing.rejected;
        records.push(outcome.record);
        if outcome.stop_run {
            return Ok(current_report(
                records,
                non_selected,
                cached,
                total_expectations,
                evaluated,
                selected,
                silent,
                narrowing,
            ));
        }
    }
    Ok(current_report(
        records,
        non_selected,
        cached,
        total_expectations,
        evaluated,
        selected,
        silent,
        narrowing,
    ))
}

fn write_cached_failures(
    cached_hits: Vec<CachedSelectionHit>,
    records: &mut Vec<CheckRecord>,
    cached: &mut Vec<CachedExpectation>,
    result_output: &mut Option<&mut dyn std::io::Write>,
    started: std::time::Instant,
) -> Result<(), String> {
    for CachedSelectionHit { expectation, hit } in cached_hits {
        let record = hit.record;
        if !record.passed() {
            write_and_flush_result_output(result_output, &record, started.elapsed())?;
            records.push(record.clone());
        }
        cached.push(CachedExpectation {
            expectation,
            record,
        });
    }
    Ok(())
}

fn current_report(
    records: Vec<CheckRecord>,
    non_selected: Vec<crate::check::core::types::SelectedExpectation>,
    cached: Vec<CachedExpectation>,
    total_expectations: usize,
    evaluated: usize,
    selected: usize,
    silent: usize,
    narrowing: NarrowingStats,
) -> CheckRunReport {
    let skipped = skipped_count(total_expectations, &records, &cached);
    check_run_report(
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
    )
}
