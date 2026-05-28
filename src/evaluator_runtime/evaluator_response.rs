use crate::check_types::{contains_line_break, EvaluatorResponseJson, ParsedAnswer};
use crate::config_types::AgentConfig;
use crate::{ERROR_INSUFFICIENT_EVIDENCE, ERROR_INVALID_QUESTION, ERROR_UNPARSABLE};
use serde::Deserialize;

pub(crate) fn parse_evaluator_response(
    text: &str,
    _agent: &AgentConfig,
) -> Result<ParsedAnswer, String> {
    let response = parse_evaluator_response_json(text)?;
    let has_answer = response.answer.is_some();
    let has_error = response.error.is_some();
    if has_answer == has_error {
        return Err("evaluator response must contain exactly one of answer or error".to_string());
    }
    if response.evidence.trim().is_empty() {
        return Err("evidence must be a non-empty string".to_string());
    }
    if let Some(answer) = response.answer {
        if answer.trim().is_empty() || contains_line_break(&answer) {
            return Err("answer must be a non-empty single-line string".to_string());
        }
        validate_q_scope_suggestion(response.q_scope_suggestion.as_deref())?;
        // Pass/fail comparison happens after parsing against the expectation's
        // current expected answer.
        return Ok(ParsedAnswer::answer(
            answer,
            response.evidence,
            response.q_scope_suggestion,
        ));
    }
    let error = response
        .error
        .expect("one-of validation ensures error is present");
    if !matches!(
        error.as_str(),
        ERROR_INSUFFICIENT_EVIDENCE | ERROR_INVALID_QUESTION | ERROR_UNPARSABLE
    ) {
        return Err(format!("unsupported evaluator error: {}", error));
    }
    if contains_line_break(&error) {
        return Err("error must be a single-line string".to_string());
    }
    Ok(ParsedAnswer::error(error, response.evidence))
}

fn validate_q_scope_suggestion(scope: Option<&[String]>) -> Result<(), String> {
    let Some(scope) = scope else {
        return Ok(());
    };
    if scope.is_empty() {
        return Err("qScopeSuggestion must contain at least one path".to_string());
    }
    for item in scope {
        if item.trim().is_empty() || contains_line_break(item) {
            return Err("qScopeSuggestion items must be non-empty single-line strings".to_string());
        }
    }
    Ok(())
}

pub(crate) fn parse_evaluator_response_json(text: &str) -> Result<EvaluatorResponseJson, String> {
    let payload = evaluator_response_json_payload(text)?;
    serde_json::from_str::<EvaluatorResponseJson>(payload)
        .map_err(|err| format!("failed to parse evaluator JSON response: {}", err))
}

pub(crate) fn evaluator_response_json_payload(text: &str) -> Result<&str, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("evaluator response must be a JSON object".to_string());
    }
    let mut deserializer = serde_json::Deserializer::from_str(trimmed);
    serde_json::Value::deserialize(&mut deserializer)
        .map_err(|err| format!("failed to inspect evaluator JSON response: {}", err))?;
    deserializer
        .end()
        .map_err(|_| "evaluator response must not contain surrounding prose".to_string())?;
    if !trimmed.starts_with('{') {
        return Err("evaluator response must be a JSON object".to_string());
    }
    Ok(trimmed)
}
