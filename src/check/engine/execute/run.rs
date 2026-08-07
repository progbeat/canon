use super::expectation::{run_selected_check_expectation, CheckExpectationRunContext};
use super::report::{check_run_report, pending_count, CheckRunReportCounts};
use super::work_queue::{prepare_selected_diff_trees, select_check_work};
use super::CheckRunSideEffects;
use crate::check::core::{
    check_run_error, CheckOptions, CheckRecord, CheckRunError, CheckRunReport,
};
use crate::check::interrogation::state::CheckRuntime;
use crate::check::interrogation::InterrogationSession;
use crate::evaluator::EvaluatorRunner;
use crate::isolation::prepare_evaluator_isolation_environment;

// This runtime layer consumes resolved check options and owns cache/evaluator
// work ordering. Command-line policy such as default-run feedback stays
// in `check::command::workflow`. It does not define a separate persistent
// state family; durable writes are delegated to xpec last-result storage and
// diagnostic runtime logs, whose modules own cleanup, fixed file sets, and
// rotation.
pub(crate) fn run_check_with_runner_and_caches<R: EvaluatorRunner>(
    mut runtime: CheckRuntime<'_>,
    options: &CheckOptions,
    runner: &mut R,
    progress_report: &mut CheckRunReport,
    side_effects: CheckRunSideEffects<'_, '_, '_, '_>,
) -> Result<CheckRunReport, CheckRunError> {
    let CheckRunSideEffects {
        mut diagnostic_log,
        mut result_output,
        live_report_output,
        caches,
        resolve_selected_diff_from_tree_oids,
    } = side_effects;
    // xpec: kK
    // Config expansion establishes the collected set. CLI selectors choose
    // candidates from that set; they do not remove unselected xpecs from it,
    // so every collected xpec without a result remains pending.
    let total_collected_expectations = runtime.config.expectations.len();
    *progress_report = current_report(Vec::new(), Vec::new(), total_collected_expectations);
    macro_rules! current_error {
        ($error:expr) => {
            check_run_error($error, progress_report.clone())
        };
    }
    macro_rules! run_try {
        ($expr:expr) => {
            $expr.map_err(|err| current_error!(err.to_string()))?
        };
    }

    let disable_session_isolation = runtime.disable_session_isolation();
    if !disable_session_isolation {
        // `canon check` preparation may create its configured evaluator
        // sandbox. `canon ask` constructs the shared run state without this
        // check-only persistent side effect.
        run_try!(prepare_evaluator_isolation_environment());
    }
    let mut interrogation_session = run_try!(InterrogationSession::new(
        disable_session_isolation,
        caches.temporary_directory_allocator.clone(),
    ));
    let check_work = run_try!(select_check_work(
        &runtime,
        options,
        caches,
        &mut diagnostic_log,
    ));
    // [HS] These records were excluded before Canon's Selected set was
    // formed. Adding them to summary bookkeeping here does not evaluate,
    // emit, or interleave them with the ordered evaluator queue.
    for hit in check_work.reused_non_selected_results {
        progress_report.cached_passes.push(hit.pass_record);
    }
    update_pending(progress_report, total_collected_expectations);
    let check_work_queue = check_work.selected_evaluation_queue;
    run_try!(prepare_selected_diff_trees(
        &mut runtime,
        &check_work_queue,
        &mut caches.repo_inspection,
        resolve_selected_diff_from_tree_oids,
    ));
    for expectation in check_work_queue {
        let outcome = match run_selected_check_expectation(
            &mut CheckExpectationRunContext {
                runtime: &runtime,
                options,
                runner,
                diagnostic_log: &mut diagnostic_log,
                result_output: &mut result_output,
                live_report_output: &live_report_output,
                caches,
                interrogation_session: &mut interrogation_session,
                progress_report,
            },
            &expectation,
        ) {
            Ok(outcome) => outcome,
            Err(error) => return Err(current_error!(error)),
        };
        let stop_run = outcome.stop_run;
        let interrupted = outcome.interrupted;
        if stop_run {
            // This is the shared default-order stop point for both
            // materialized and in-place runs. In-place mode changes cached
            // result eligibility, not the stop-after-evaluated-fail rule.
            let report = progress_report.clone();
            if interrupted {
                // The interruption still finishes through the normal partial
                // report path. Default-source runs emit the feedback required
                // by AL after the summary, including pending expectations.
                return Err(check_run_error(
                    "check interrupted after the current expectation".to_string(),
                    report,
                ));
            }
            return Ok(report);
        }
    }
    Ok(progress_report.clone())
}

fn update_pending(report: &mut CheckRunReport, total_collected_expectations: usize) {
    report.pending = pending_count(
        total_collected_expectations,
        &report.records,
        &report.cached_passes,
    );
}

fn current_report(
    records: Vec<CheckRecord>,
    cached_passes: Vec<crate::check::core::CachedPassRecord>,
    total_collected_expectations: usize,
) -> CheckRunReport {
    let pending = pending_count(total_collected_expectations, &records, &cached_passes);
    check_run_report(records, cached_passes, CheckRunReportCounts { pending })
}
