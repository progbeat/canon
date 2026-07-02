use super::expectation::{run_expectation, ExpectationRunContext};
use super::report::{check_run_report, skipped_count, CheckRunReportCounts};
use super::CheckRunSideEffects;
use crate::check::command::output::write_cached_non_pass_output;
use crate::check::core::{
    check_run_error, interrupted_check_run_error, CachedExpectation, CheckOptions, CheckRecord,
    CheckRunError, CheckRunReport, SelectedExpectation,
};
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::check::run::selection::{
    order_by_latest_non_pass, select_git_backed_expectations_after_cache, CachedExpectationHit,
    CachedNonPassPolicy, GitBackedCacheFilterContext,
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

    let mut interrogation_run_state = run_try!(InterrogationRunState::new(
        runtime.no_sandbox() || runtime.is_in_place()
    ));
    let check_work_queue = if runtime.is_in_place() {
        // In-place config compatibility has already been validated by the
        // command layer, and this runtime has no Git-backed cache source.
        // Every candidate goes straight to evaluator work in stable order.
        evaluate_only_check_work_queue(options.pre_cache_candidates.clone())
    } else {
        caches.run_start_pass_ids = run_try!(snapshot_pass_ids(
            root,
            &options.pre_cache_candidates,
            &mut caches.xpec_state,
        ));
        let source = runtime
            .tree_source()
            .ok_or_else(|| current_error!("missing Git tree source".to_string()))?;
        // Explicit selectors are still routed through this component, but the
        // cache selector returns before any cache lookup when
        // `selectors_provided` is true. That keeps forced selections in the
        // evaluator queue even if reusable cached results exist.
        let check_work = run_try!(select_git_backed_expectations_after_cache(
            GitBackedCacheFilterContext {
                root,
                source,
                xpec_state: &mut caches.xpec_state,
                visible_tree_oid_cache: &mut caches.visible_tree_oid,
                diagnostic_log: &mut diagnostic_log,
            },
            options,
            run_try!(unix_timestamp()),
            if options.selectors_provided {
                CachedNonPassPolicy::EvaluateUncached
            } else {
                CachedNonPassPolicy::LeaveUncachedPending
            },
        ));
        // Cache selection returns cached report work plus the evaluator queue.
        // Sort both by latest non-pass timestamp so public output and evaluator
        // work share one priority order.
        run_try!(order_check_work(
            root,
            check_work.cached_hits,
            check_work.evaluation_queue,
            &mut caches.xpec_state,
        ))
    };
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
                let stop_run = outcome.stop_run;
                let interrupted = outcome.interrupted;
                // This is the structured per-expectation report for evaluated
                // work; human stdout/stderr rendering is not the only report.
                records.push(outcome.record);
                if stop_run {
                    // This is the shared default-order stop point for both
                    // materialized and in-place runs. In-place mode changes
                    // cache/state usage, not the stop-after-evaluated-non-pass
                    // rule.
                    if !interrupted {
                        write_remaining_cached_failures_without_evaluation(
                            &mut check_work_queue,
                            &mut cached,
                            &mut result_output,
                        )
                        .map_err(|err| current_error!(err))?;
                    }
                    let report = current_report(records, cached, total_expectations);
                    if interrupted {
                        // The post-summary agent-message spec is explicitly scoped to
                        // runs without Ctrl-C or other interruption. Resource/control
                        // stop signals finish through the error-report path so no
                        // commit/fix instruction is printed for a partial run.
                        return Err(interrupted_check_run_error(
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

fn write_remaining_cached_failures_without_evaluation(
    items: &mut impl Iterator<Item = CheckWorkItem>,
    cached: &mut Vec<CachedExpectation>,
    result_output: &mut Option<&mut dyn std::io::Write>,
) -> Result<(), String> {
    // The order spec stops evaluator work after the first evaluated non-pass.
    // Remaining cached failures are already-known results, so writing their
    // public blocks does not evaluate another selected expectation.
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

fn evaluate_only_check_work_queue(
    evaluation_queue: Vec<SelectedExpectation>,
) -> Vec<CheckWorkItem> {
    evaluation_queue
        .into_iter()
        .map(|expectation| CheckWorkItem::Evaluate(Box::new(expectation)))
        .collect()
}

fn order_check_work(
    root: &Path,
    cached_hits: Vec<CachedExpectationHit>,
    evaluation_queue: Vec<SelectedExpectation>,
    xpec_state: &mut crate::xpec_state::XpecStateCache,
) -> Result<Vec<CheckWorkItem>, String> {
    let work = cached_hits
        .into_iter()
        .map(|hit| CheckWorkItem::Cached(Box::new(hit)))
        .chain(
            evaluation_queue
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
        // Cached passes are counted in the summary only; they are not emitted
        // as per-expectation stdout result entries and therefore have no
        // progress timeline. Cache selection excludes human-review last-error
        // records, so the only cached results with a printed short ID are
        // failed answers, and this branch writes their complete public block.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_types::{AgentConfig, DEFAULT_DIFF_FROM};

    #[test]
    fn evaluate_only_check_work_queue_preserves_candidate_order() {
        let queue = evaluate_only_check_work_queue(vec![
            selected_expectation("first"),
            selected_expectation("second"),
        ]);

        let ids = queue
            .into_iter()
            .map(|item| match item {
                CheckWorkItem::Evaluate(expectation) => expectation.id,
                CheckWorkItem::Cached(_) => {
                    panic!("evaluate-only queue must not contain cache hits")
                }
            })
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["first", "second"]);
    }

    fn selected_expectation(id: &str) -> SelectedExpectation {
        SelectedExpectation {
            number: 1,
            id: id.to_string(),
            display_id: id.to_string(),
            question: "Does it pass?".to_string(),
            expected_answer: "yes".to_string(),
            question_context: String::new(),
            diff_from: DEFAULT_DIFF_FROM.to_string(),
            target: None,
            question_answer_only: false,
            agent: AgentConfig::default(),
            cooldown: None,
        }
    }
}
