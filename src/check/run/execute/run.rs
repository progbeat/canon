use super::expectation::{run_expectation, ExpectationRunContext};
use super::report::{check_run_report, skipped_count, CheckRunReportCounts};
use super::CheckRunSideEffects;
use crate::check::command::output::write_cached_non_pass_output;
use crate::check::core::{
    check_run_error, CachedExpectation, CheckOptions, CheckRecord, CheckRunError, CheckRunReport,
};
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::check::run::selection::{
    order_by_latest_non_pass, select_expectations_after_cache, CacheFilterContext,
    CachedExpectationHit, CachedFailureMode,
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
        live_report_output,
        caches,
    } = side_effects;
    let mut records = Vec::new();
    let mut cached = Vec::new();
    let total_expectations = runtime.config.expectations.len();
    let mut evaluated = 0usize;
    let root = runtime.root;
    macro_rules! current_error {
        ($error:expr) => {
            check_run_error(
                $error,
                check_run_report(
                    records.clone(),
                    cached.clone(),
                    CheckRunReportCounts {
                        evaluated,
                        skipped: skipped_count(total_expectations, &records, &cached),
                    },
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
    let check_work = run_try!(select_expectations_after_cache(
        CacheFilterContext {
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
    write_cached_failures(check_work.cached_hits, &mut cached, &mut result_output)
        .map_err(|err| current_error!(err))?;
    if check_work.cached_failure_seen
        && check_work.to_evaluate.is_empty()
        && !options.selectors_provided
    {
        return Ok(current_report(
            records,
            cached,
            total_expectations,
            evaluated,
        ));
    }
    let check_work_queue = run_try!(order_by_latest_non_pass(
        root,
        check_work.to_evaluate,
        &mut caches.history,
        |expectation| expectation
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
                live_report_output: &live_report_output,
                caches,
                interrogation_run_state: &mut interrogation_run_state,
            },
            expectation,
        ) {
            Ok(outcome) => outcome,
            Err(error) => return Err(current_error!(error)),
        };
        evaluated += 1;
        records.push(outcome.record);
        if outcome.stop_run {
            return Ok(current_report(
                records,
                cached,
                total_expectations,
                evaluated,
            ));
        }
    }
    Ok(current_report(
        records,
        cached,
        total_expectations,
        evaluated,
    ))
}

fn write_cached_failures(
    cached_hits: Vec<CachedExpectationHit>,
    cached: &mut Vec<CachedExpectation>,
    result_output: &mut Option<&mut dyn std::io::Write>,
) -> Result<(), String> {
    for CachedExpectationHit { expectation, hit } in cached_hits {
        let record = hit.record;
        // Cached passes are summary-only results; they do not start a displayed
        // expectation report because no `<short ID>.` prefix is printed for
        // them. The only cached results with a printed short ID are
        // non-passes, and this branch writes their complete public block.
        let cached_result_prints_short_id = !record.passed();
        if cached_result_prints_short_id {
            write_cached_non_pass_output(result_output, &record)?;
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
    cached: Vec<CachedExpectation>,
    total_expectations: usize,
    evaluated: usize,
) -> CheckRunReport {
    let skipped = skipped_count(total_expectations, &records, &cached);
    check_run_report(records, cached, CheckRunReportCounts { evaluated, skipped })
}
