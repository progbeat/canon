use super::expectation::{run_expectation, ExpectationRunContext};
use super::report::{check_run_report, skipped_count, CheckRunReportCounts};
use super::CheckRunSideEffects;
use crate::check::command::output::write_cached_non_pass_output;
use crate::check::core::{
    check_run_error, CachedExpectation, CheckOptions, CheckRecord, CheckRunError, CheckRunReport,
    SelectedExpectation,
};
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::check::run::selection::{
    order_by_latest_non_pass, select_expectations_after_cache, CacheFilterContext,
    CachedExpectationHit, CachedFailureMode,
};
use crate::evaluator::EvaluatorRunner;
use crate::time::unix_timestamp;
use crate::xpec_state::snapshot_pass_ids;
use std::path::Path;

// This runtime layer consumes resolved check options and owns cache/evaluator
// work ordering. Command-line policy such as default-run agent messaging stays
// in `check::command::execution`.
pub(crate) fn run_check_with_runner_and_caches<R: EvaluatorRunner>(
    runtime: CheckRuntime<'_>,
    options: &CheckOptions,
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
    let root = runtime.root;
    macro_rules! current_error {
        ($error:expr) => {
            check_run_error(
                $error,
                check_run_report(
                    records.clone(),
                    cached.clone(),
                    CheckRunReportCounts {
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
    caches.run_start_pass_ids = run_try!(snapshot_pass_ids(
        root,
        &options.selected,
        &mut caches.xpec_state,
    ));
    let check_work = run_try!(select_expectations_after_cache(
        CacheFilterContext {
            root,
            source: runtime.tree_source,
            xpec_state: &mut caches.xpec_state,
            visible_tree_oid_cache: &mut caches.visible_tree_oid,
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
    // Cached hits reuse results while evaluate items still require evaluator
    // work, but both are ordered together for the public check run.
    let check_work_queue = run_try!(order_check_work(
        root,
        check_work.cached_hits,
        check_work.to_evaluate,
        &mut caches.xpec_state,
    ));
    let mut check_work_queue = check_work_queue.into_iter();
    while let Some(item) = check_work_queue.next() {
        match item {
            CheckWorkItem::Cached(hit) => {
                write_cached_failures(vec![*hit], &mut cached, &mut result_output)
                    .map_err(|err| current_error!(err))?;
            }
            CheckWorkItem::Evaluate(expectation) => {
                let expectation = *expectation;
                let outcome = match run_expectation(
                    &mut ExpectationRunContext {
                        runtime: &runtime,
                        options,
                        runner,
                        diagnostic_log: &mut diagnostic_log,
                        result_output: &mut result_output,
                        live_report_output: &live_report_output,
                        caches,
                        interrogation_run_state: &mut interrogation_run_state,
                    },
                    &expectation,
                ) {
                    Ok(outcome) => outcome,
                    Err(error) => return Err(current_error!(error)),
                };
                records.push(outcome.record);
                if outcome.stop_run {
                    if !outcome.interrupted {
                        write_remaining_cached_work_items(
                            &mut check_work_queue,
                            &mut cached,
                            &mut result_output,
                        )
                        .map_err(|err| current_error!(err))?;
                    }
                    let report = current_report(records, cached, total_expectations);
                    if outcome.interrupted {
                        // The post-summary agent-message spec is explicitly scoped to
                        // runs without Ctrl-C or other interruption. Resource/control
                        // stop signals finish through the error-report path so no
                        // commit/fix instruction is printed for a partial run.
                        return Err(check_run_error(
                            "check interrupted after the current expectation".to_string(),
                            report,
                        ));
                    }
                    return Ok(report);
                }
            }
        }
    }
    Ok(current_report(records, cached, total_expectations))
}

fn write_remaining_cached_work_items(
    items: &mut impl Iterator<Item = CheckWorkItem>,
    cached: &mut Vec<CachedExpectation>,
    result_output: &mut Option<&mut dyn std::io::Write>,
) -> Result<(), String> {
    for item in items {
        if let CheckWorkItem::Cached(hit) = item {
            write_cached_failures(vec![*hit], cached, result_output)?;
        }
    }
    Ok(())
}

enum CheckWorkItem {
    Cached(Box<CachedExpectationHit>),
    Evaluate(Box<SelectedExpectation>),
}

fn order_check_work(
    root: &Path,
    cached_hits: Vec<CachedExpectationHit>,
    to_evaluate: Vec<SelectedExpectation>,
    xpec_state: &mut crate::xpec_state::XpecStateCache,
) -> Result<Vec<CheckWorkItem>, String> {
    let work = cached_hits
        .into_iter()
        .map(|hit| CheckWorkItem::Cached(Box::new(hit)))
        .chain(
            to_evaluate
                .into_iter()
                .map(|expectation| CheckWorkItem::Evaluate(Box::new(expectation))),
        )
        .collect::<Vec<_>>();
    order_by_latest_non_pass(root, work, xpec_state, |item| match item {
        CheckWorkItem::Cached(hit) => &hit.expectation,
        CheckWorkItem::Evaluate(expectation) => expectation,
    })
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
        // them. Cache selection excludes human-review last-error records, so
        // the only cached results with a printed short ID are failed answers,
        // and this branch writes their complete public block.
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
) -> CheckRunReport {
    let skipped = skipped_count(total_expectations, &records, &cached);
    check_run_report(records, cached, CheckRunReportCounts { skipped })
}
