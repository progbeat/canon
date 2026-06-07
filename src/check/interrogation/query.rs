mod narrowing;
mod review;
mod turn;

use crate::check::core::types::QueryResult;
use crate::check::interrogation::model_fallback::run_with_model_fallbacks;
use crate::check::interrogation::records::{
    write_query_result_event, write_query_review_required_event,
};
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::evaluator::{EvaluatorError, EvaluatorRunner};
use crate::logs::DiagnosticLogWriter;
use crate::scope::sanitize_scope;

#[derive(Clone, Copy)]
pub(crate) struct QueryRequest<'a> {
    pub(crate) question: &'a str,
    pub(crate) enforced_scope: &'a [String],
}

pub(crate) fn run_query_with_runner<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    enforced_scope: &[String],
    runner: &mut R,
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
) -> Result<QueryResult, String> {
    // Query lifecycle start/finish events are emitted by `check::command::query`
    // so they bracket scope parsing and execution preparation as well as the
    // evaluator turn managed here.
    let mut diagnostic_log = diagnostic_log;
    run_with_model_fallbacks(
        &runtime.config.agent,
        state,
        &mut diagnostic_log,
        None,
        |state, diagnostic_log, model| {
            ask_with_model(
                runtime,
                QueryRequest {
                    question,
                    enforced_scope,
                },
                runner,
                diagnostic_log,
                state,
                model,
            )
        },
    )
}

fn ask_with_model<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    query: QueryRequest<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    model: Option<&str>,
) -> Result<QueryResult, EvaluatorError> {
    // `canon check -q` uses the same evaluator input shape as normal checks.
    // q-scope suggestions are trusted only after an independent verification
    // turn returns a schema-valid answer under the suggested scope.
    let mut active_scope = query.enforced_scope.to_vec();
    let mut result = turn::ask_with_full_scope_retry(
        runtime,
        query.question,
        &mut active_scope,
        runner,
        diagnostic_log,
        state,
        model,
    )?;
    if narrowing::should_verify(runtime, state, &active_scope, &result.answer)? {
        let Some(suggestion) = result.answer.q_scope_suggestion.as_deref() else {
            return Err(EvaluatorError::message(
                "q-scope verification requested without a suggestion",
            ));
        };
        let proposed_scope = sanitize_scope(suggestion)?;
        let mut verification_scope = proposed_scope.clone();
        let narrowed = turn::ask_with_full_scope_retry(
            runtime,
            query.question,
            &mut verification_scope,
            runner,
            diagnostic_log,
            state,
            model,
        )?;
        if narrowing::answer_is_accepted(&narrowed.answer, &proposed_scope) {
            result = narrowed;
        }
        result.answer.q_scope_suggestion = None;
    }
    if let Some(reason) = review::human_review_reason(&result) {
        write_query_review_required_event(query.question, diagnostic_log, &result.answer, reason)?;
        return Err(EvaluatorError::message(format!(
            "query requires human review: {}",
            reason
        )));
    }
    write_query_result_event(query.question, diagnostic_log, &result.answer)?;
    Ok(result)
}
