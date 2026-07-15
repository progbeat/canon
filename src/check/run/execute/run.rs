use super::expectation::{run_expectation, ExpectationRunContext};
use super::report::{check_run_report, skipped_count, CheckRunReportCounts};
use super::CheckRunSideEffects;
use crate::check::core::{
    check_run_error, interrupted_check_run_error, CachedExpectation, CheckOptions, CheckRecord,
    CheckRunError, CheckRunReport, ResolvedExpectation,
};
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::check::run::selection::{
    order_by_latest_fail, order_in_place_by_absent_fail_history,
    select_git_backed_expectations_after_cache, GitBackedCacheFilterContext,
};
use crate::evaluator::EvaluatorRunner;
use crate::time::unix_timestamp;
use crate::xpec_state::snapshot_pass_ids;

// This runtime layer consumes resolved check options and owns cache/evaluator
// work ordering. Command-line policy such as default-run agent messaging stays
// in `check::command::execution`. It does not define a separate persistent
// state family; durable writes are delegated to xpec last-result storage and
// diagnostic runtime logs, whose modules own cleanup, fixed file sets, and
// rotation.
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
        // The canon check --in-place spec treats persisted xpec history as
        // absent, so e5 ordering uses the Unix epoch for every candidate.
        in_place_check_work_queue(options.candidate_expectations.clone())
    } else {
        caches.run_start_pass_ids = run_try!(snapshot_pass_ids(
            root,
            &options.candidate_expectations,
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
        ));
        for hit in check_work.cached_hits {
            cached.push(CachedExpectation {
                expectation: hit.expectation,
                record: hit.hit.record,
            });
        }
        run_try!(order_by_latest_fail(
            root,
            check_work.evaluation_queue,
            &mut caches.xpec_state,
            |expectation| expectation,
        ))
    };
    for expectation in check_work_queue {
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
            // cache/state usage, not the stop-after-evaluated-fail
            // rule.
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
    Ok(current_report(records, cached, total_expectations))
}

fn in_place_check_work_queue(
    evaluation_queue: Vec<ResolvedExpectation>,
) -> Vec<ResolvedExpectation> {
    order_in_place_by_absent_fail_history(evaluation_queue, |expectation| expectation)
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

    // xpec: Un,Mx
    #[test]
    fn in_place_check_work_queue_uses_in_place_absent_history_epoch_order() {
        let queue = in_place_check_work_queue(vec![
            resolved_expectation("first"),
            resolved_expectation("second"),
        ]);

        let ids = queue
            .into_iter()
            .map(|expectation| expectation.id)
            .collect::<Vec<_>>();

        // xpec: Un,Mx
        assert_eq!(ids, vec!["first", "second"]);
    }

    fn resolved_expectation(id: &str) -> ResolvedExpectation {
        ResolvedExpectation {
            number: 1,
            id: id.to_string(),
            display_id: id.to_string(),
            to: crate::config_types::ExpectationTo::Agent,
            rank: 0,
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
