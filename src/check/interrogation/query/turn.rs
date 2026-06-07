use crate::check::core::types::QueryResult;
use crate::check::interrogation::records::finalize_query_answer;
use crate::check::interrogation::state::{
    should_retry_full_scope_after_error, CheckRuntime, InterrogationRunState,
};
use crate::check::interrogation::{ask_with_reused_thread, ThreadTurnRequest};
use crate::evaluator::{evaluator_turn_prompt, EvaluatorError, EvaluatorRunner};
use crate::hash::full_scope;
use crate::logs::DiagnosticLogWriter;

pub(super) fn ask_with_full_scope_retry<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    enforced_scope: &mut Vec<String>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    model: Option<&str>,
) -> Result<QueryResult, EvaluatorError> {
    let mut result = ask_once(
        runtime,
        question,
        enforced_scope,
        runner,
        diagnostic_log,
        state,
        model,
    )?;
    if should_retry_full_scope_after_error(result.answer.error.as_deref(), enforced_scope) {
        // Restricted insufficient-evidence is not final for query-mode
        // interrogations either; retry once with full project scope.
        *enforced_scope = full_scope();
        result = ask_once(
            runtime,
            question,
            enforced_scope,
            runner,
            diagnostic_log,
            state,
            model,
        )?;
    }
    Ok(result)
}

fn ask_once<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    enforced_scope: &[String],
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    model: Option<&str>,
) -> Result<QueryResult, EvaluatorError> {
    let prompt = evaluator_turn_prompt(question, None)?;
    let response = ask_with_reused_thread(
        runtime,
        runner,
        diagnostic_log,
        state,
        ThreadTurnRequest {
            agent: &runtime.config.agent,
            enforced_scope,
            model,
            thinking: &runtime.config.agent.thinking,
            expectation_id: None,
            prompt: &prompt,
        },
    )?;
    finalize_query_answer(runtime, state, enforced_scope, question, response.answer)
}
