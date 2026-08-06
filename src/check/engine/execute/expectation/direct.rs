use super::finish::{
    append_check_result_to_user_visible_report, record_finished_check_expectation,
    write_user_visible_caller_check_result,
};
use super::{
    assert_final_check_evaluation_postconditions, CheckExpectationRunContext,
    CheckExpectationRunOutcome,
};
use crate::check::command::output::render_caller_prompt;
use crate::check::core::{
    CheckRecord, CheckRecordOutcome, CheckResult, EvaluationAnswer, ResolvedExpectation,
};
use crate::check::engine::execute::persistence::FinishedCheckRecordSource;
use crate::config_types::ExpectationTo;
use crate::evaluator::EvaluatorRunner;
use crate::hash::full_scope;
use std::io::BufRead;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::time::Instant;

const PANICKED_DIRECT_EVALUATION_ERROR: &str = "evaluation panicked";

pub(super) fn run_direct_check_expectation<R: EvaluatorRunner>(
    context: &mut CheckExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
) -> Result<CheckExpectationRunOutcome, String> {
    let scope = full_scope();
    let visible_tree_oid = context.cached_visible_tree_oid(expectation, &scope)?;
    let mut started_report = if expectation.to == ExpectationTo::Shell {
        context
            .live_report_output
            .as_ref()
            .map(|output| {
                super::super::progress::start_live_expectation_report(None, output, expectation)
            })
            .transpose()?
    } else {
        None
    };
    let evaluation_started_at = Instant::now();
    let response = catch_unwind(AssertUnwindSafe(|| match expectation.to {
        ExpectationTo::Caller => evaluate_caller(context.result_output, expectation),
        ExpectationTo::Shell => {
            super::super::shell::evaluate(context.runtime.root, &expectation.question)
                .map(|evaluation| (evaluation.answer, evaluation.transcript))
        }
        ExpectationTo::Agent => unreachable!("agent xpecs use interrogation"),
    }));
    let response = match response {
        Ok(response) => response,
        Err(payload) => {
            // [Eg] Caller and shell evaluations share the agent evaluator's
            // BaseException boundary: synthesize and report a final FAIL/error
            // record, stop any started shell timeline, then preserve the
            // original panic for the command-level finally handler.
            let _ = catch_unwind(AssertUnwindSafe(|| {
                finish_panicked_direct_check_expectation(
                    context,
                    expectation,
                    visible_tree_oid.clone(),
                    &mut started_report,
                    evaluation_started_at.elapsed(),
                )
            }));
            resume_unwind(payload)
        }
    };
    let evaluation_elapsed = evaluation_started_at.elapsed();
    let (observed, evidence, error) = match response {
        Ok((answer, evidence)) => (
            answer.into_string(),
            (expectation.to == ExpectationTo::Shell).then_some(evidence),
            None,
        ),
        Err(error) => (String::new(), None, Some(error)),
    };
    let result =
        CheckResult::from_evaluation(expectation.expected_answer(), &observed, error.as_deref());
    let outcome = CheckRecordOutcome::new(result, observed, error, evidence, scope);
    let record = CheckRecord::current_from_expectation(
        expectation,
        CheckRecordOutcome {
            visible_tree_oid,
            ..outcome
        },
    )?;
    assert_final_check_evaluation_postconditions(&record);
    if let Some(started_report) = started_report.take() {
        append_check_result_to_user_visible_report(started_report, &record);
    } else {
        write_user_visible_caller_check_result(context.result_output, &record, evaluation_elapsed)?;
    }
    context.record_completed(&record);
    record_finished_check_expectation(
        context,
        expectation,
        &record,
        FinishedCheckRecordSource::DirectEvaluation,
    )?;
    Ok(CheckExpectationRunOutcome::after_evaluation(
        &record,
        context.options.keep_going,
        false,
    ))
}

fn finish_panicked_direct_check_expectation<R: EvaluatorRunner>(
    context: &mut CheckExpectationRunContext<'_, '_, '_, R>,
    expectation: &ResolvedExpectation,
    visible_tree_oid: Option<String>,
    started_report: &mut Option<super::super::progress::LiveExpectationReport>,
    evaluation_elapsed: std::time::Duration,
) -> Result<(), String> {
    let result = CheckResult::from_evaluation(
        expectation.expected_answer(),
        "",
        Some(PANICKED_DIRECT_EVALUATION_ERROR),
    );
    let outcome = CheckRecordOutcome::new(
        result,
        String::new(),
        Some(PANICKED_DIRECT_EVALUATION_ERROR.to_string()),
        None,
        full_scope(),
    );
    let record = CheckRecord::current_from_expectation(
        expectation,
        CheckRecordOutcome {
            visible_tree_oid,
            ..outcome
        },
    )?;
    assert_final_check_evaluation_postconditions(&record);
    if let Some(started_report) = started_report.take() {
        append_check_result_to_user_visible_report(started_report, &record);
    } else {
        write_user_visible_caller_check_result(context.result_output, &record, evaluation_elapsed)?;
    }
    context.record_completed(&record);
    record_finished_check_expectation(
        context,
        expectation,
        &record,
        FinishedCheckRecordSource::DirectEvaluation,
    )
}

fn evaluate_caller(
    result_output: &mut Option<&mut dyn std::io::Write>,
    expectation: &ResolvedExpectation,
) -> Result<(EvaluationAnswer, String), String> {
    if let Some(output) = result_output.as_mut() {
        let prompt = render_caller_prompt(&expectation.question);
        crate::check::command::output::write_stdout_record(
            *output,
            prompt.as_bytes(),
            "caller xpec prompt",
        )?;
    }
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let answer = read_caller_answer(&mut input)?;
    // [MH] Terminal input is already text; preserve it exactly when entering
    // the shared evaluation-response string domain.
    Ok((EvaluationAnswer::new(answer), String::new()))
}

fn read_caller_answer(input: &mut impl BufRead) -> Result<String, String> {
    let mut answer = String::new();
    let bytes_read = input
        .read_line(&mut answer)
        .map_err(|error| format!("failed to read caller answer: {error}"))?;
    if bytes_read == 0 {
        return Err("failed to read caller answer: end of input".to_string());
    }
    trim_line_ending(&mut answer);
    Ok(answer)
}

fn trim_line_ending(line: &mut String) {
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
}
