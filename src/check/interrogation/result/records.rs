use crate::check::core::{
    CheckRecord, InterrogationAnswer, InterrogationResult, ParsedAnswer, ResolvedExpectation,
};
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::evaluator::{record_from_response, EvaluatorError};
use crate::logs::{DiagnosticLogWriter, DiagnosticRecordEvent};
use crate::scope::sanitize_scope;
use serde_json::json;

pub(crate) fn interrogation_result_from_answer(
    expectation: &ResolvedExpectation,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    answer: InterrogationAnswer,
) -> Result<InterrogationResult, EvaluatorError> {
    let InterrogationAnswer {
        answer,
        visible_tree_oid,
        diff_from,
        diff_from_tree_oid,
        diff_from_tree_oid_abbrev,
        context_compacted,
        interrupted,
    } = answer;
    let record = record_from_response(
        expectation,
        answer,
        visible_tree_oid,
        diff_from,
        diff_from_tree_oid,
        diff_from_tree_oid_abbrev,
    )?;
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_record_event(DiagnosticRecordEvent::Interrogation, &record)?;
    }
    Ok(InterrogationResult {
        record,
        context_compacted,
        interrupted,
    })
}

pub(crate) fn finalize_interrogation_answer(
    runtime: &CheckRuntime<'_>,
    state: &mut InterrogationRunState,
    agent: &crate::config_types::AgentConfig,
    enforced_scope: &[String],
    response: ParsedAnswer,
    context_compacted: bool,
) -> Result<InterrogationAnswer, EvaluatorError> {
    // InterrogationAnswer is invocation-local normalized evaluator output.
    // Normal check expectations convert it to CheckRecord; `canon ask` reports
    // it directly and never reaches durable xpec last-result storage.
    let finalized = finalize_parsed_answer(runtime, state, agent, enforced_scope, response)?;
    Ok(InterrogationAnswer {
        answer: finalized.response,
        visible_tree_oid: finalized.visible_tree_oid,
        diff_from: None,
        diff_from_tree_oid: None,
        diff_from_tree_oid_abbrev: None,
        context_compacted,
        interrupted: false,
    })
}

pub(crate) fn write_query_result_event(
    question: &str,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    answer: &ParsedAnswer,
) -> Result<(), EvaluatorError> {
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.emit_event(
            "info",
            "query.result",
            &[
                ("prompt", json!(question)),
                ("observed", json!(answer.observed.clone())),
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

// Result records are only one runtime-log family. Normal check paths call
// these writers for evaluated and cached expectations; `canon ask` writes
// query result/review events instead. Evaluator boundary events such as thread
// creation/reuse, restart, agent request/response/failure, and per-turn token
// usage are emitted by `check::interrogation::session` through the same
// `DiagnosticLogWriter`.
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
        writer.emit_event(
            "warn",
            "query.review_required",
            &[
                ("prompt", json!(question)),
                ("observed", json!(answer.observed.clone())),
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
    visible_tree_oid: Option<String>,
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
        visible_tree_oid,
    })
}
