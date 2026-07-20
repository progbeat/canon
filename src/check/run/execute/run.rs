use super::expectation::{run_expectation, ExpectationRunContext};
use super::report::{check_run_report, pending_count, CheckRunReportCounts};
use super::CheckRunSideEffects;
use crate::check::core::{
    check_run_error, CheckOptions, CheckRecord, CheckRunError, CheckRunReport, ResolvedExpectation,
};
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::check::run::selection::{
    order_selected_by_rank_and_latest_fail,
    order_selected_when_every_expectation_has_no_fail_result,
    select_and_order_git_backed_expectations, GitBackedCacheFilterContext,
};
use crate::evaluator::EvaluatorRunner;
use crate::isolation::prepare_evaluator_isolation_environment;
use crate::time::unix_timestamp;
use std::path::Path;

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
    // xpec: 9b
    // Config expansion establishes the collected set. CLI selectors choose
    // candidates from that set; they do not remove unselected xpecs from it,
    // so every collected xpec without a result remains pending.
    let total_collected_expectations = runtime.config.expectations.len();
    let root = runtime.root;
    macro_rules! current_error {
        ($error:expr) => {
            check_run_error(
                $error,
                check_run_report(
                    records.clone(),
                    cached.clone(),
                    CheckRunReportCounts {
                        pending: pending_count(total_collected_expectations, &records, &cached),
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

    let disable_session_isolation = runtime.disable_session_isolation();
    if !disable_session_isolation {
        // `canon check` preparation may create its configured evaluator
        // sandbox. `canon ask` constructs the shared run state without this
        // check-only persistent side effect.
        run_try!(prepare_evaluator_isolation_environment());
    }
    let mut interrogation_run_state =
        run_try!(InterrogationRunState::new(disable_session_isolation));
    let check_work_queue = if runtime.is_in_place() {
        // The in-place selection boundary proves why every candidate is
        // Selected before applying the common order policy.
        if runtime.persistent_check_state_root().is_some() {
            run_try!(select_and_order_in_place_expectations(
                root,
                options.candidate_expectations.clone(),
                &mut caches.xpec_state,
            ))
        } else {
            select_and_order_in_place_expectations_without_state(
                options.candidate_expectations.clone(),
            )
        }
    } else {
        let source = runtime
            .tree_source()
            .ok_or_else(|| current_error!("missing Git tree source".to_string()))?;
        // Git-backed selection owns both cache filtering and final evaluation
        // ordering. Explicit selectors skip cache lookup but are still sorted
        // by rank and latest fail before this function receives the queue.
        let check_work = run_try!(select_and_order_git_backed_expectations(
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
        // [iZ] These records were excluded before Canon's Selected set was
        // formed. Adding them to summary bookkeeping here does not evaluate,
        // emit, or interleave them with the ordered evaluator queue.
        for hit in check_work.reused_non_selected_results {
            cached.push(hit.record);
        }
        check_work.selected_evaluation_queue
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
            // materialized and in-place runs. In-place mode changes cached
            // result eligibility, not the stop-after-evaluated-fail rule.
            let report = current_report(records, cached, total_collected_expectations);
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
    Ok(current_report(
        records,
        cached,
        total_collected_expectations,
    ))
}

fn select_and_order_in_place_expectations(
    // This function contains only platform-independent selection ordering.
    // Filesystem and process variants stay behind platform-named modules.
    root: &Path,
    candidates: Vec<ResolvedExpectation>,
    last_result_history: &mut crate::xpec_state::XpecStateCache,
) -> Result<Vec<ResolvedExpectation>, String> {
    // [eM,iY,cg,uf,I4] Cached Result is defined only for an expectation plus
    // Git state. In-place mode has no Git state, so its Cached set is
    // structurally empty and both default and explicit selection retain every
    // CLI candidate. Persisted last-fail history affects only their order.
    order_selected_by_rank_and_latest_fail(root, candidates, last_result_history, |expectation| {
        expectation
    })
}

fn select_and_order_in_place_expectations_without_state(
    candidates: Vec<ResolvedExpectation>,
) -> Vec<ResolvedExpectation> {
    // [eM,iY,cg,IJ,I4] In-place still has no Cached Result domain. Without a
    // persistent state root it also has no last-result namespace, so every
    // selected candidate has the Unix epoch as its absent fail timestamp.
    order_selected_when_every_expectation_has_no_fail_result(candidates, |expectation| expectation)
}

fn current_report(
    records: Vec<CheckRecord>,
    cached: Vec<CheckRecord>,
    total_collected_expectations: usize,
) -> CheckRunReport {
    let pending = pending_count(total_collected_expectations, &records, &cached);
    check_run_report(records, cached, CheckRunReportCounts { pending })
}

#[cfg(test)]
mod tests {
    use super::select_and_order_in_place_expectations;
    use crate::check::core::{CheckRecord, CheckResult, ResolvedExpectation};
    use crate::config_types::{AgentConfig, ExpectationTo, DEFAULT_DIFF_FROM};
    use crate::hash::full_scope;
    use crate::xpec_state::XpecStateCache;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: eM,iY,cg,uf,IJ,I4,gD
    fn in_place_has_no_cached_results_and_selects_every_candidate() {
        let root = test_root("in-place-order-history");
        fs::create_dir_all(&root).unwrap();
        let status = Command::new("git").arg("init").arg(&root).status().unwrap();
        assert!(status.success());
        let older = resolved_expectation("older");
        let newer = resolved_expectation("newer");
        let mut last_result_history = XpecStateCache::default();
        write_in_place_fail(&root, &older, 1, &mut last_result_history);
        write_in_place_fail(&root, &newer, 2, &mut last_result_history);

        let queue = select_and_order_in_place_expectations(
            &root,
            vec![older, newer],
            &mut XpecStateCache::default(),
        )
        .unwrap();
        let ids = queue
            .into_iter()
            .map(|expectation| expectation.id)
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["newer", "older"]);
        let _ = fs::remove_dir_all(root);
    }

    fn write_in_place_fail(
        root: &Path,
        expectation: &ResolvedExpectation,
        timestamp: u64,
        last_result_history: &mut XpecStateCache,
    ) {
        let record = CheckRecord {
            timestamp: crate::time::format_record_timestamp(timestamp),
            number: expectation.number,
            result: CheckResult::Fail,
            to: expectation.to,
            question: Some(expectation.question.clone()),
            expected_answer: Some(expectation.expected_answer.clone()),
            observed: "no".to_string(),
            error: None,
            evidence: Some(String::new()),
            scope: full_scope(),
            question_scope_suggestion: None,
            visible_tree_oid: None,
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
        };
        last_result_history
            .write_last_result_for_record(root, None, expectation, &record)
            .unwrap();
    }

    fn resolved_expectation(id: &str) -> ResolvedExpectation {
        ResolvedExpectation {
            number: 1,
            id: id.to_string(),
            display_id: id.to_string(),
            to: ExpectationTo::Agent,
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

    fn test_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("canon-{name}-{}-{unique}", process::id()))
    }
}
