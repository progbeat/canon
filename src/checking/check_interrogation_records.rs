use crate::check_interrogation_state::{CheckRuntime, InterrogationState};
use crate::check_types::{InterrogationResult, ParsedAnswer, QueryResult, SelectedExpectation};
use crate::evaluator_turn::{record_from_response, ParsedTurnResponse};
use crate::evaluator_types::EvaluatorError;
use crate::hash::full_scope;
use crate::logging::DiagnosticLogWriter;
use crate::scope::{normalize_repo_path, sanitize_scope, scope_is_within};
use crate::{EMPTY_EVIDENCE_OBSERVED, OBSERVED_IDK, OBSERVED_MALFORMED, UNPARSEABLE_OBSERVED};
use serde_json::json;

const EMPTY_EVIDENCE_MESSAGE: &str = "evaluator response evidence was empty";

pub(crate) fn finalize_interrogation_response(
    runtime: &CheckRuntime<'_>,
    expectation: &SelectedExpectation,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
    enforced_scope: &[String],
    turn_response: ParsedTurnResponse,
) -> Result<InterrogationResult, EvaluatorError> {
    let finalized = finalize_parsed_answer(runtime, state, enforced_scope, turn_response.answer)?;
    let record_scope = finalized.response.scope.clone();
    let record = record_from_response(
        &runtime.config.agent,
        expectation,
        finalized.response,
        record_scope,
        finalized.visible_tree_oid,
    )?;
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_interrogation_record(&record)?;
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
    state: &mut InterrogationState,
    enforced_scope: &[String],
    response: ParsedAnswer,
) -> Result<QueryResult, EvaluatorError> {
    let finalized = finalize_parsed_answer(runtime, state, enforced_scope, response)?;
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
                ("scope", json!(answer.scope.clone())),
            ],
        )?;
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
                ("scope", json!(answer.scope.clone())),
                ("reason", json!(reason)),
            ],
        )?;
    }
    Ok(())
}

struct FinalizedParsedAnswer {
    response: ParsedAnswer,
    visible_tree_oid: String,
}

fn finalize_parsed_answer(
    runtime: &CheckRuntime<'_>,
    state: &mut InterrogationState,
    enforced_scope: &[String],
    response: ParsedAnswer,
) -> Result<FinalizedParsedAnswer, EvaluatorError> {
    let mut response = normalize_empty_evidence_response(response, enforced_scope);
    response = normalize_missing_evidence_citation_response(response);
    response = enforce_response_scope(response, enforced_scope);
    response = reject_absent_response_scope(runtime, state, enforced_scope, response)?;
    if response.answer == UNPARSEABLE_OBSERVED {
        response.scope = enforced_scope.to_vec();
    }
    // Evaluator parsing normalizes scopes before this point; normalize again
    // after local repairs such as widened-scope rejection so stored records and
    // hashes use the same canonical representation.
    response.scope = sanitize_scope(&response.scope, &runtime.config.agent)?;
    let visible_tree_oid = state.visible_tree_oid_cache.staged_visible_tree_oid(
        runtime.root,
        &runtime.config.agent,
        &response.scope,
    )?;
    Ok(FinalizedParsedAnswer {
        response,
        visible_tree_oid,
    })
}

fn reject_absent_response_scope(
    runtime: &CheckRuntime<'_>,
    state: &mut InterrogationState,
    enforced_scope: &[String],
    response: ParsedAnswer,
) -> Result<ParsedAnswer, EvaluatorError> {
    if response.answer == UNPARSEABLE_OBSERVED {
        return Ok(response);
    }
    let missing = state
        .visible_tree_oid_cache
        .missing_staged_scope_paths(runtime.root, &response.scope)?;
    if missing.is_empty() {
        return Ok(response);
    }
    let message = format!(
        "evaluator response scope entries are absent from the staged snapshot: {}",
        missing.join(", ")
    );
    Ok(ParsedAnswer {
        answer: UNPARSEABLE_OBSERVED.to_string(),
        evidence: response_evidence_with_message(&response.evidence, &message),
        scope: enforced_scope.to_vec(),
    })
}

pub(crate) fn enforce_response_scope(
    response: ParsedAnswer,
    enforced_scope: &[String],
) -> ParsedAnswer {
    if response.answer == UNPARSEABLE_OBSERVED || scope_is_within(&response.scope, enforced_scope) {
        return response;
    }
    if is_terminal_review_answer(&response.answer) {
        let evidence = rejected_widened_scope_evidence(&response, enforced_scope);
        return ParsedAnswer {
            answer: response.answer,
            evidence,
            scope: enforced_scope.to_vec(),
        };
    }
    if enforced_scope != full_scope() {
        // Reject the evaluator-proposed widened scope, but turn the restricted
        // response into an `idk` non-answer so the caller can perform the
        // interrogation-policy full-scope retry instead of converting a scope
        // formatting mistake into a terminal review record.
        let evidence = rejected_widened_scope_evidence(&response, enforced_scope);
        return ParsedAnswer {
            answer: OBSERVED_IDK.to_string(),
            evidence,
            scope: enforced_scope.to_vec(),
        };
    }
    ParsedAnswer {
        answer: UNPARSEABLE_OBSERVED.to_string(),
        evidence: rejected_widened_scope_message(&response.scope, enforced_scope),
        scope: enforced_scope.to_vec(),
    }
}

fn is_terminal_review_answer(answer: &str) -> bool {
    answer == EMPTY_EVIDENCE_OBSERVED || answer == OBSERVED_MALFORMED
}

fn rejected_widened_scope_evidence(response: &ParsedAnswer, enforced_scope: &[String]) -> String {
    let message = rejected_widened_scope_message(&response.scope, enforced_scope);
    response_evidence_with_message(&response.evidence, &message)
}

fn rejected_widened_scope_message(response_scope: &[String], enforced_scope: &[String]) -> String {
    format!(
        "evaluator response scope {:?} widens enforced scope {:?}",
        response_scope, enforced_scope
    )
}

fn response_evidence_with_message(evidence: &str, message: &str) -> String {
    if evidence.trim().is_empty() {
        message.to_string()
    } else {
        format!("{}\n{}", evidence, message)
    }
}

fn normalize_empty_evidence_response(
    response: ParsedAnswer,
    enforced_scope: &[String],
) -> ParsedAnswer {
    if response.evidence.trim().is_empty() && response.answer != UNPARSEABLE_OBSERVED {
        if response.answer == OBSERVED_IDK && enforced_scope != full_scope() {
            return ParsedAnswer {
                answer: OBSERVED_IDK.to_string(),
                evidence: EMPTY_EVIDENCE_MESSAGE.to_string(),
                scope: response.scope,
            };
        }
        return ParsedAnswer {
            answer: EMPTY_EVIDENCE_OBSERVED.to_string(),
            evidence: EMPTY_EVIDENCE_MESSAGE.to_string(),
            scope: response.scope,
        };
    }
    response
}

fn normalize_missing_evidence_citation_response(response: ParsedAnswer) -> ParsedAnswer {
    if response.answer == EMPTY_EVIDENCE_OBSERVED
        || (response.answer == OBSERVED_IDK && response.evidence == EMPTY_EVIDENCE_MESSAGE)
        || response.answer == UNPARSEABLE_OBSERVED
        || evidence_has_project_citation(&response.evidence)
    {
        return response;
    }
    ParsedAnswer {
        answer: OBSERVED_MALFORMED.to_string(),
        evidence: response_evidence_with_message(
            &response.evidence,
            "evaluator response evidence did not contain a backticked project-relative citation",
        ),
        scope: response.scope,
    }
}

fn evidence_has_project_citation(evidence: &str) -> bool {
    let mut rest = evidence;
    while let Some(start) = rest.find('`') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('`') else {
            return false;
        };
        if is_project_citation(&rest[..end]) {
            return true;
        }
        rest = &rest[end + 1..];
    }
    false
}

fn is_project_citation(citation: &str) -> bool {
    let path = citation
        .split_once(':')
        .map(|(path, _line)| path)
        .unwrap_or(citation);
    if path.is_empty() || path == "." || path.starts_with('/') || path.contains("://") {
        return false;
    }
    normalize_repo_path(path).is_ok()
}
