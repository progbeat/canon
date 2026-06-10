use crate::check::core::{
    CheckRecord, InterrogationResult, ParsedAnswer, QueryResult, SelectedExpectation,
};
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::evaluator::{record_from_response, EvaluatorError, ParsedTurnResponse};
use crate::logs::{DiagnosticLogWriter, DiagnosticRecordEvent};
use crate::scope::sanitize_scope;
use serde_json::json;

pub(crate) fn finalize_interrogation_response(
    runtime: &CheckRuntime<'_>,
    expectation: &SelectedExpectation,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    enforced_scope: &[String],
    turn_response: ParsedTurnResponse,
) -> Result<InterrogationResult, EvaluatorError> {
    let finalized = finalize_parsed_answer(
        runtime,
        state,
        &expectation.agent,
        enforced_scope,
        turn_response.answer,
    )?;
    let record = record_from_response(
        expectation,
        finalized.response,
        finalized.scope,
        finalized.visible_tree_oid,
    )?;
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_record_event(DiagnosticRecordEvent::Interrogation, &record)?;
    }
    Ok(InterrogationResult {
        record,
        turn_usage: turn_response.usage,
        context_compacted: turn_response.context_compacted,
        stop_after_current_expectation: false,
    })
}

pub(crate) fn finalize_query_answer(
    runtime: &CheckRuntime<'_>,
    state: &mut InterrogationRunState,
    enforced_scope: &[String],
    _question: &str,
    response: ParsedAnswer,
) -> Result<QueryResult, EvaluatorError> {
    let finalized = finalize_parsed_answer(
        runtime,
        state,
        &runtime.config.agent,
        enforced_scope,
        response,
    )?;
    Ok(QueryResult {
        answer: finalized.response,
    })
}

pub(crate) fn write_query_result_event(
    question: &str,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    answer: &ParsedAnswer,
) -> Result<(), EvaluatorError> {
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_event(
            "info",
            "query.result",
            &[
                ("prompt", json!(question)),
                ("observed", json!(answer.answer.clone())),
                ("evidence", json!(answer.evidence.clone())),
                (
                    "qScopeSuggestion",
                    json!(answer.question_scope_suggestion.clone()),
                ),
            ],
        )?;
    }
    Ok(())
}

// Result records are only one runtime-log family. Evaluator boundary events
// such as thread creation/reuse, restart, agent request/response/failure, and
// per-turn token usage are emitted by `check::interrogation::session` through
// the same `DiagnosticLogWriter`.
pub(crate) fn write_expectation_result_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    record: &CheckRecord,
) -> Result<(), String> {
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer
            .write_record_event(DiagnosticRecordEvent::Expectation, record)
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

pub(crate) fn write_query_review_required_event(
    question: &str,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    answer: &ParsedAnswer,
    reason: &str,
) -> Result<(), EvaluatorError> {
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_event(
            "warn",
            "query.review_required",
            &[
                ("prompt", json!(question)),
                ("observed", json!(answer.answer.clone())),
                ("evidence", json!(answer.evidence.clone())),
                (
                    "qScopeSuggestion",
                    json!(answer.question_scope_suggestion.clone()),
                ),
                ("reason", json!(reason)),
            ],
        )?;
    }
    Ok(())
}

struct FinalizedParsedAnswer {
    response: ParsedAnswer,
    scope: Vec<String>,
    visible_tree_oid: String,
}

fn finalize_parsed_answer(
    runtime: &CheckRuntime<'_>,
    state: &mut InterrogationRunState,
    agent: &crate::config_types::AgentConfig,
    enforced_scope: &[String],
    response: ParsedAnswer,
) -> Result<FinalizedParsedAnswer, EvaluatorError> {
    let scope = sanitize_scope(enforced_scope)?;
    let visible_tree_oid =
        runtime.visible_tree_oid(&mut state.visible_tree_oid_cache, agent, &scope)?;
    let mut response = response;
    response.scope = scope.clone();
    Ok(FinalizedParsedAnswer {
        response,
        scope,
        visible_tree_oid,
    })
}
