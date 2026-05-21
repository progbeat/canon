use crate::check_interrogation::{ask_with_reused_thread, ThreadTurnRequest};
use crate::check_interrogation_records::{finalize_query_answer, write_query_result_event};
use crate::check_interrogation_state::{CheckRuntime, InterrogationState};
use crate::check_model_fallback::run_with_model_fallbacks;
use crate::check_types::{ObservedAnswerState, QueryResult};
use crate::evaluator_types::{EvaluatorError, EvaluatorRunner};
use crate::logging::DiagnosticLogWriter;
use crate::scope::is_strict_scope_subset;
use crate::OBSERVED_IDK;

pub(crate) fn run_query_with_runner<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    enforced_scope: &[String],
    runner: &mut R,
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
) -> Result<QueryResult, String> {
    let mut diagnostic_log = diagnostic_log;
    run_with_model_fallbacks(
        &runtime.config.agent,
        state,
        &mut diagnostic_log,
        None,
        |state, diagnostic_log, model| {
            ask_query_with_model(
                runtime,
                question,
                enforced_scope,
                runner,
                diagnostic_log,
                state,
                model,
            )
        },
    )
}

pub(crate) fn ask_query_with_model<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    enforced_scope: &[String],
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
    model: Option<&str>,
) -> Result<QueryResult, EvaluatorError> {
    // Query mode has no expected answer, so "incorrect" has no meaning here.
    // It still follows the expectation-interrogation control flow that does not
    // need one: restricted `idk` retries at full scope, and strict narrowed
    // scopes are accepted only after an independent query returns the same
    // reusable answer.
    let mut current_scope = enforced_scope.to_vec();
    let mut result = ask_query_once(
        runtime,
        question,
        &current_scope,
        runner,
        diagnostic_log,
        state,
        model,
    )?;
    if should_retry_query_full_scope(&result, &current_scope) {
        current_scope = crate::hash::full_scope();
        result = ask_query_once(
            runtime,
            question,
            &current_scope,
            runner,
            diagnostic_log,
            state,
            model,
        )?;
    }
    if should_verify_query_narrowing(&result, &current_scope) {
        let narrowed_scope = result.answer.scope.clone();
        let narrowed = ask_query_once(
            runtime,
            question,
            &narrowed_scope,
            runner,
            diagnostic_log,
            state,
            model,
        )?;
        if narrowed_query_answer_is_accepted(&result, &narrowed) {
            result = narrowed;
        }
    }
    reject_query_human_review(&result, &current_scope)?;
    write_query_result_event(question, diagnostic_log, &result.answer)?;
    Ok(result)
}

fn ask_query_once<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    enforced_scope: &[String],
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
    model: Option<&str>,
) -> Result<QueryResult, EvaluatorError> {
    let prompt = question.to_string();
    let response = ask_with_reused_thread(
        runtime,
        runner,
        diagnostic_log,
        state,
        ThreadTurnRequest {
            enforced_scope,
            model,
            thinking: &runtime.config.agent.thinking,
            expectation_id: None,
            prompt: &prompt,
        },
    )?;
    finalize_query_answer(runtime, state, enforced_scope, response.answer)
}

fn should_retry_query_full_scope(result: &QueryResult, enforced_scope: &[String]) -> bool {
    result.answer.answer == OBSERVED_IDK && enforced_scope != crate::hash::full_scope()
}

fn should_verify_query_narrowing(result: &QueryResult, enforced_scope: &[String]) -> bool {
    ObservedAnswerState::from_observed(&result.answer.answer).is_reusable_history()
        && is_strict_scope_subset(&result.answer.scope, enforced_scope)
}

fn narrowed_query_answer_is_accepted(wide: &QueryResult, narrowed: &QueryResult) -> bool {
    ObservedAnswerState::from_observed(&narrowed.answer.answer).is_reusable_history()
        && narrowed.answer.answer == wide.answer.answer
}

fn reject_query_human_review(
    result: &QueryResult,
    enforced_scope: &[String],
) -> Result<(), EvaluatorError> {
    let reason = match ObservedAnswerState::from_observed(&result.answer.answer) {
        ObservedAnswerState::Idk if enforced_scope == crate::hash::full_scope() => {
            Some("full-scope idk")
        }
        ObservedAnswerState::Malformed => Some("malformed evaluator response"),
        ObservedAnswerState::Unparseable => Some("unparseable evaluator response"),
        ObservedAnswerState::EmptyEvidence => Some("empty evaluator evidence"),
        ObservedAnswerState::Unknown => Some("unknown observed answer state"),
        ObservedAnswerState::Idk | ObservedAnswerState::Answer => None,
    };
    if let Some(reason) = reason {
        return Err(EvaluatorError::message(format!(
            "query requires human review: {}",
            reason
        )));
    }
    Ok(())
}
