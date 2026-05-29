use crate::check_types::{EvaluatorResponseJson, ParsedAnswer};
use crate::config_types::AgentConfig;
use serde::Deserialize;
use serde_json::Value;

pub(crate) fn parse_evaluator_response(
    text: &str,
    _agent: &AgentConfig,
) -> Result<ParsedAnswer, String> {
    let response = parse_evaluator_response_json(text)?;
    response.validate_schema()?;
    // The Interrogation Policy schema permits qScopeSuggestion on the raw
    // response object, but narrowing policy only consumes it from schema-valid
    // answer responses. Error responses are review records and never seed
    // narrowing, answer history, or q-scope reuse.
    if let Some(answer) = response.answer {
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
    Ok(ParsedAnswer::error(error, response.evidence))
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
