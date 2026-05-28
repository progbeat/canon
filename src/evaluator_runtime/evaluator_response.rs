use crate::check_types::{EvaluatorResponseJson, ParsedAnswer};
use crate::config_types::AgentConfig;
use crate::{ERROR_INSUFFICIENT_EVIDENCE, ERROR_INVALID_QUESTION, ERROR_UNPARSABLE};
use serde::Deserialize;
use serde_json::Value;

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
    validate_q_scope_suggestion(response.q_scope_suggestion.as_deref())?;
    if let Some(answer) = response.answer {
        if answer.is_empty() || contains_schema_line_break(&answer) {
            return Err("answer must be a non-empty single-line string".to_string());
        }
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
        if item.is_empty() || contains_schema_line_break(item) {
            return Err("qScopeSuggestion items must be non-empty single-line strings".to_string());
        }
    }
    Ok(())
}

fn contains_schema_line_break(value: &str) -> bool {
    value
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'\r' | b'\n'))
}

pub(crate) fn parse_evaluator_response_json(text: &str) -> Result<EvaluatorResponseJson, String> {
    let payload = evaluator_response_json_payload(text)?;
    let raw = serde_json::from_str::<Value>(payload)
        .map_err(|err| format!("failed to parse evaluator JSON response: {}", err))?;
    reject_explicit_null_schema_fields(&raw)?;
    serde_json::from_value::<EvaluatorResponseJson>(raw)
        .map_err(|err| format!("failed to parse evaluator JSON response: {}", err))
}

fn reject_explicit_null_schema_fields(raw: &Value) -> Result<(), String> {
    let Some(object) = raw.as_object() else {
        return Err("evaluator response must be a JSON object".to_string());
    };
    for key in ["answer", "error", "qScopeSuggestion"] {
        if object.get(key).is_some_and(Value::is_null) {
            return Err(format!("{} must not be null", key));
        }
    }
    Ok(())
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
