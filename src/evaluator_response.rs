use crate::check_types::{contains_line_break, EvaluatorResponseJson, ParsedAnswer};
use crate::config_types::AgentConfig;
use crate::evaluator_json::validate_evaluator_response_key_order;
use crate::evaluator_scope::parse_scope_strings;

pub(crate) fn parse_evaluator_response(
    text: &str,
    agent: &AgentConfig,
) -> Result<ParsedAnswer, String> {
    let response = parse_evaluator_response_json(text)?;
    if contains_line_break(&response.answer) {
        return Err("answer must be a single-line string".to_string());
    }
    // Parsed answers stay vocabulary-neutral. `ObservedAnswerState` validates
    // the answer shape against the expectation: yes/no questions reject
    // free-form prose, while free-form expectations can compare exact strings.
    Ok(ParsedAnswer {
        answer: response.answer,
        evidence: response.evidence,
        scope: parse_scope_strings(&response.scope, agent)?,
    })
}

pub(crate) fn parse_evaluator_response_json(text: &str) -> Result<EvaluatorResponseJson, String> {
    let payload = evaluator_response_json_payload(text)?;
    serde_json::from_str::<EvaluatorResponseJson>(payload)
        .map_err(|err| format!("failed to parse evaluator JSON response: {}", err))
}

pub(crate) fn evaluator_response_json_payload(text: &str) -> Result<&str, String> {
    let trimmed = text.trim();
    validate_evaluator_response_key_order(trimmed)?;
    Ok(trimmed)
}
