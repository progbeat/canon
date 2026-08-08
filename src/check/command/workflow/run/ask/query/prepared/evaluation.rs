use super::output::finish_query_output_and_collect_token_usage_summary;
use super::AskQueryError;
use crate::app::LazyAppServerRunner;
use crate::check::command::output::{start_query_report_output, SharedCheckOutput};
use crate::check::command::{run_with_token_usage_panic_capture, TokenUsageSummary};
use crate::check::core::{evaluate_final_response, ParsedAnswer, QueryResult};
use crate::check::interrogation::query::{
    run_query_with_runner, QueryExpectationContext, QueryRequest,
};
use crate::check::interrogation::state::CheckRuntime;
use crate::check::interrogation::InterrogationSession;
use crate::check::{CheckRunCaches, ResolvedExpectation};
use crate::config_types::{CheckConfig, ExpectationTo};
use crate::evaluator::EvaluatorRunner;
use crate::logs::DiagnosticLogWriter;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

const PANICKED_ASK_EVALUATION_ERROR: &str = "evaluation panicked";

#[allow(clippy::too_many_arguments)]
pub(super) fn evaluate_prepared_ask_query(
    runtime: CheckRuntime<'_>,
    runner: &mut LazyAppServerRunner,
    question: &str,
    config: &CheckConfig,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    check_caches: &mut CheckRunCaches,
    token_usage_summary: &mut TokenUsageSummary,
) -> Result<(), AskQueryError> {
    let mut completed_token_usage = TokenUsageSummary::unavailable();
    let result = run_with_token_usage_panic_capture(runner, token_usage_summary, |runner| {
        evaluate_prepared_ask_query_with_runner(
            runtime,
            runner,
            question,
            config,
            diagnostic_log,
            check_caches,
            &mut completed_token_usage,
        )
    });
    *token_usage_summary = completed_token_usage;
    result
}

#[allow(clippy::too_many_arguments)]
fn evaluate_prepared_ask_query_with_runner(
    runtime: CheckRuntime<'_>,
    runner: &mut LazyAppServerRunner,
    question: &str,
    config: &CheckConfig,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    check_caches: &mut CheckRunCaches,
    token_usage_summary: &mut TokenUsageSummary,
) -> Result<(), AskQueryError> {
    // xpec: l
    assert!(
        runtime.persistent_check_state_root().is_none(),
        "prepared ask runtime must not expose persistent xpec history"
    );
    // This is the single canonical ask xpec after ordinary item resolution:
    // ask supplied to/q/a explicitly, presets supplied only omitted fields,
    // and configured check xpecs were excluded.
    let [resolved_ask_xpec] = config.expectations.as_slice() else {
        return Err("ask config must contain exactly one temporary expectation"
            .to_string()
            .into());
    };
    // xpec: l
    assert_eq!(
        resolved_ask_xpec.q.as_str(),
        question,
        "the canonical ask xpec must preserve the command question"
    );
    // xpec: Eg,l
    assert_eq!(
        resolved_ask_xpec.to,
        ExpectationTo::Agent,
        "the canonical ask xpec must address the agent evaluator"
    );
    // xpec: l
    assert_eq!(
        resolved_ask_xpec.a, "",
        "the canonical ask xpec must have an empty expected answer"
    );
    let temporary_expectation = ResolvedExpectation::from_resolved_ask_xpec(resolved_ask_xpec);
    // xpec: l
    assert!(
        runtime.scope_without_reusable_q_scope_history().is_some(),
        "a temporary query has no reusable q-scope history"
    );
    let enforced_scope =
        crate::check::q_scope::initial_q_scope_without_history(&temporary_expectation);
    let expectation = QueryExpectationContext {
        expectation: &temporary_expectation,
    };
    // This is the `canon ask` evaluator boundary: every prepared ask creates a
    // temporary resultless xpec and sends it through the same interrogation path
    // as check evaluation. There is no cache hit or last-result shortcut here.
    let mut interrogation_session = InterrogationSession::new(
        runtime.disable_session_isolation(),
        check_caches.temporary_directory_allocator.clone(),
    )?;
    let shared_output = SharedCheckOutput::stdout();
    let started_report =
        start_query_report_output(shared_output, &temporary_expectation.display_id);
    let progress = started_report.progress();
    runner.set_progress_reporter(Some(progress.clone()));
    let result = catch_unwind(AssertUnwindSafe(|| {
        run_query_with_runner(
            &runtime,
            QueryRequest {
                question: &temporary_expectation.question,
                enforced_scope: &enforced_scope,
                expectation,
                progress: Some(&progress),
            },
            runner,
            diagnostic_log.as_deref_mut(),
            &mut interrogation_session,
            check_caches,
        )
    }));
    runner.set_progress_reporter(None);
    let result = match result {
        Err(payload) => {
            // [Eg,l] BaseException still finishes the ask evaluator's timeline,
            // computes its failed status, and reports its error before the
            // original panic resumes. A reporting panic must not mask it.
            let panic_result = finished_ask_error_result(
                temporary_expectation.expected_answer(),
                PANICKED_ASK_EVALUATION_ERROR.to_string(),
            );
            let _ = catch_unwind(AssertUnwindSafe(|| {
                finish_query_output_and_collect_token_usage_summary(
                    started_report,
                    &panic_result,
                    None,
                    runner,
                    token_usage_summary,
                )
            }));
            resume_unwind(payload)
        }
        Ok(result) => result,
    };
    let result = match result {
        Ok(result) => finish_ask_result(temporary_expectation.expected_answer(), result),
        Err(err) => {
            let result =
                finished_ask_error_result(temporary_expectation.expected_answer(), err.clone());
            return match finish_query_output_and_collect_token_usage_summary(
                started_report,
                &result,
                None,
                runner,
                token_usage_summary,
            ) {
                Ok(()) => Err(AskQueryError::Reported(err)),
                Err(output_err) => Err(output_err),
            };
        }
    };
    let human_review_reason = result.human_review_reason();
    finish_query_output_and_collect_token_usage_summary(
        started_report,
        &result,
        human_review_reason,
        runner,
        token_usage_summary,
    )?;
    if let Some(reason) = human_review_reason {
        return Err(AskQueryError::Reported(format!(
            "query requires human review: {reason}"
        )));
    }
    Ok(())
}

fn finished_ask_error_result(expected_answer: &str, error: String) -> QueryResult {
    finish_ask_result(
        expected_answer,
        QueryResult {
            answer: ParsedAnswer::error_without_evidence(error),
            diff_from: None,
            diff_from_tree_oid_abbrev: None,
        },
    )
}

fn finish_ask_result(expected_answer: &str, result: QueryResult) -> QueryResult {
    // [Eg,l] Every successful or error `canon ask` acquisition converges here
    // before output. The canonical evaluator postconditions are therefore
    // explicit at the command boundary that owns the final response.
    let _status = evaluate_final_response(
        expected_answer,
        &result.answer.observed,
        result.answer.error.as_deref(),
    );
    result
}
