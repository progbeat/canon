use crate::check::core::{
    CheckRecord, InterrogationAnswer, InterrogationAnswerData, InterrogationResult,
    InterrogationTurn, ParsedAnswer, ResolvedExpectation,
};
use crate::check::interrogation::state::CheckRuntime;
use crate::evaluator::{record_from_response, EvaluatorError};
use crate::git::VisibleTreeOidCache;
use crate::logs::{DiagnosticLogWriter, DiagnosticRecordEvent};
use crate::scope::sanitize_scope;
use serde_json::json;

pub(crate) fn interrogation_result_from_answer(
    expectation: &ResolvedExpectation,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    answer: InterrogationAnswer,
) -> Result<InterrogationResult, EvaluatorError> {
    let InterrogationTurn {
        output:
            InterrogationAnswerData {
                answer,
                visible_tree_oid,
                diff_from,
                diff_from_tree_oid,
                diff_from_tree_oid_abbrev,
            },
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
    Ok(InterrogationResult::new(
        record,
        context_compacted,
        interrupted,
    ))
}

pub(crate) fn finalize_interrogation_answer(
    runtime: &CheckRuntime<'_>,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    agent: &crate::config_types::AgentConfig,
    enforced_scope: &[String],
    response: ParsedAnswer,
    context_compacted: bool,
) -> Result<InterrogationAnswer, EvaluatorError> {
    // InterrogationAnswer is invocation-local normalized evaluator output.
    // Normal check expectations convert it to CheckRecord; `canon ask` reports
    // it directly and never reaches durable xpec last-result storage.
    let finalized = finalize_parsed_answer(
        runtime,
        visible_tree_oid_cache,
        agent,
        enforced_scope,
        response,
    )?;
    Ok(InterrogationAnswer::new(
        InterrogationAnswerData {
            answer: finalized.response,
            visible_tree_oid: finalized.visible_tree_oid,
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
        },
        context_compacted,
        false,
    ))
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
            &query_event_fields(question, answer),
        )?;
    }
    Ok(())
}

// [kK] Result records are only one runtime-log family. Normal check finish and
// cache paths call this writer after raw evaluator responses have been logged;
// `DiagnosticLogWriter::write_record_event` emits the parsed expectation
// outcome and, when applicable, its review-required diagnostic. `canon ask`
// writes query result/review events instead. Evaluator boundary events such as
// thread creation/reuse, restart, agent request/response/failure, and per-turn
// token usage are emitted by `check::interrogation::session` through the same
// writer.
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
        let mut fields = query_event_fields(question, answer);
        fields.push(("reason", json!(reason)));
        writer.emit_event("warn", "query.review_required", &fields)?;
    }
    Ok(())
}

fn query_event_fields(
    question: &str,
    answer: &ParsedAnswer,
) -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("prompt", json!(question)),
        ("observed", json!(answer.observed)),
        ("evidence", json!(answer.evidence)),
        ("qScopeSuggestion", json!(answer.q_scope_suggestion)),
    ]
}

struct FinalizedParsedAnswer {
    response: ParsedAnswer,
    visible_tree_oid: Option<String>,
}

fn finalize_parsed_answer(
    runtime: &CheckRuntime<'_>,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    agent: &crate::config_types::AgentConfig,
    enforced_scope: &[String],
    response: ParsedAnswer,
) -> Result<FinalizedParsedAnswer, EvaluatorError> {
    let scope = sanitize_scope(enforced_scope)?;
    let visible_tree_oid = runtime.visible_tree_oid(visible_tree_oid_cache, agent, &scope)?;
    let mut response = response;
    response.scope = scope;
    Ok(FinalizedParsedAnswer {
        response,
        visible_tree_oid,
    })
}
