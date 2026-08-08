mod selected_schema_validation;

use super::{EvaluatorResponseSchemaScope, ParsedAnswer};
use selected_schema_validation::AgentTurnEnvelopeJson;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) use selected_schema_validation::UnvalidatedAgentResultJson;

pub(crate) fn parse_evaluator_response_for_short_id(
    text: &str,
    schema_scope: EvaluatorResponseSchemaScope,
    short_id: &str,
    answered_short_ids: &[String],
) -> Result<ParsedAnswer, EvaluatorResponseParseError> {
    let response = parse_evaluator_response_json(text, short_id, answered_short_ids)?;
    response
        .into_schema_valid_parsed_answer(schema_scope)
        .map_err(EvaluatorResponseParseError::Schema)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EvaluatorResponseParseError {
    Schema(String),
    ShortIdResponse(String),
}

impl EvaluatorResponseParseError {
    pub(crate) fn is_short_id_response_error(&self) -> bool {
        matches!(self, EvaluatorResponseParseError::ShortIdResponse(_))
    }
}

impl std::fmt::Display for EvaluatorResponseParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvaluatorResponseParseError::Schema(error)
            | EvaluatorResponseParseError::ShortIdResponse(error) => formatter.write_str(error),
        }
    }
}

pub(crate) fn parse_evaluator_response_json(
    text: &str,
    short_id: &str,
    answered_short_ids: &[String],
) -> Result<UnvalidatedAgentResultJson, EvaluatorResponseParseError> {
    let mut responses = parse_evaluator_response_json_for_exact_requested_short_ids(
        text,
        &[short_id],
        answered_short_ids,
    )?;
    Ok(responses
        .remove(short_id)
        .expect("requested short ID was checked above"))
}

/// Parses one turn response against the complete short-ID set named by its task input.
pub(crate) fn parse_evaluator_response_json_for_exact_requested_short_ids(
    text: &str,
    requested_short_ids: &[&str],
    answered_short_ids: &[String],
) -> Result<BTreeMap<String, UnvalidatedAgentResultJson>, EvaluatorResponseParseError> {
    if requested_short_ids.is_empty() {
        return Err(EvaluatorResponseParseError::ShortIdResponse(
            "evaluator response requested no short IDs".to_string(),
        ));
    }
    let requested = requested_short_ids
        .iter()
        .map(|short_id| {
            if matches_short_id_pattern(short_id) {
                Ok((*short_id).to_string())
            } else {
                Err(EvaluatorResponseParseError::Schema(format!(
                    "evaluator response short ID must match pattern ^[A-Za-z0-9]+$: {}",
                    short_id
                )))
            }
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if requested.len() != requested_short_ids.len() {
        return Err(EvaluatorResponseParseError::ShortIdResponse(
            "evaluator response requested duplicate short IDs".to_string(),
        ));
    }
    let payload = evaluator_response_json_payload(text)?;
    // [T5,qv] Preserve each result as raw JSON while inspecting the outer map.
    // This lets short-ID mismatches win over malformed result payloads without
    // collapsing either outer duplicate keys or inner duplicate fields through
    // a generic JSON value.
    let mut deserializer = serde_json::Deserializer::from_str(payload);
    let AgentTurnEnvelopeJson {
        responses: mut raw_responses,
        duplicate_short_id,
    } = AgentTurnEnvelopeJson::deserialize(&mut deserializer).map_err(|err| {
        EvaluatorResponseParseError::Schema(format!(
            "failed to parse evaluator JSON response: {}",
            err
        ))
    })?;
    deserializer.end().map_err(|_| {
        EvaluatorResponseParseError::Schema(
            "evaluator response must not contain surrounding prose".to_string(),
        )
    })?;
    if let Some(short_id) = duplicate_short_id {
        return Err(EvaluatorResponseParseError::ShortIdResponse(format!(
            "duplicate evaluator response short ID `{}`",
            short_id
        )));
    }
    for key in raw_responses.keys() {
        if !matches_short_id_pattern(key) {
            return Err(EvaluatorResponseParseError::ShortIdResponse(format!(
                "evaluator response short ID must match pattern ^[A-Za-z0-9]+$: {}",
                key
            )));
        }
    }
    let answered = answered_short_ids.iter().cloned().collect::<BTreeSet<_>>();
    for key in raw_responses.keys() {
        if answered.contains(key) {
            return Err(EvaluatorResponseParseError::ShortIdResponse(format!(
                "evaluator response returned already answered short ID `{}`",
                key
            )));
        }
    }
    for short_id in &requested {
        if !raw_responses.contains_key(short_id) {
            return Err(EvaluatorResponseParseError::ShortIdResponse(format!(
                "evaluator response did not contain short ID `{}`",
                short_id
            )));
        }
    }
    // [qv] This is the post-transport enforcement boundary. If an object with a
    // valid key outside this turn's exact requested set reaches parsing, that
    // key answers a different interrogation. Classify it as the same short-ID
    // mismatch as a missing requested key rather than silently accepting an
    // unsolicited result. A structured transport may enforce the equivalent
    // exact-key restriction before an invalid object is returned at all.
    if let Some(key) = raw_responses
        .keys()
        .find(|key| !requested.contains(key.as_str()))
    {
        return Err(EvaluatorResponseParseError::ShortIdResponse(format!(
            "evaluator response returned unrequested short ID `{}`",
            key
        )));
    }
    let mut responses = BTreeMap::new();
    for short_id in requested_short_ids {
        let raw_response = raw_responses
            .remove(*short_id)
            .expect("requested short ID was checked above");
        let response = serde_json::from_str::<UnvalidatedAgentResultJson>(raw_response.get())
            .map_err(|err| {
                EvaluatorResponseParseError::Schema(format!(
                    "failed to parse evaluator JSON response: {}",
                    err
                ))
            })?;
        responses.insert((*short_id).to_string(), response);
    }
    Ok(responses)
}

pub(crate) fn evaluator_response_json_payload(
    text: &str,
) -> Result<&str, EvaluatorResponseParseError> {
    let trimmed = text.trim();
    if trimmed.is_empty() || !trimmed.starts_with('{') {
        return Err(EvaluatorResponseParseError::Schema(
            "evaluator response must be a JSON object".to_string(),
        ));
    }
    Ok(trimmed)
}

pub(crate) fn matches_answer_pattern(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| matches!(byte, b'-' | b'_' | b'0'..=b'9' | b'a'..=b'z'))
}

pub(super) fn matches_short_id_pattern(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::super::contract::evaluator_response_output_schema_for_exact_requested_short_ids;
    use super::super::EvaluatorResponseSchemaScope;
    use super::*;
    use serde_json::json;

    fn keyed_response(result_json: &str) -> String {
        format!(r#"{{"q":{}}}"#, result_json)
    }

    fn parse_keyed_response_json(
        result_json: &str,
    ) -> Result<UnvalidatedAgentResultJson, EvaluatorResponseParseError> {
        parse_evaluator_response_json(&keyed_response(result_json), "q", &[])
    }

    #[test] // xpec: qv
    fn evaluator_response_requires_the_requested_short_id() {
        let error = parse_evaluator_response_for_short_id(
            &keyed_response(
                r#"{"answer":"yes","evidence":"`src/main.rs:1`","qScopeSuggestion":["."]}"#,
            ),
            EvaluatorResponseSchemaScope::AutoRestricted,
            "other",
            &[],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            EvaluatorResponseParseError::ShortIdResponse(_)
        ));
        assert!(error.to_string().contains("other"));
    }

    #[test] // xpec: qv
    fn evaluator_response_rejects_already_answered_short_id_before_parsing_its_payload() {
        let error = parse_evaluator_response_for_short_id(
            r#"{
                "q":{"answer":"yes","evidence":"`src/main.rs:1`","qScopeSuggestion":["."]},
                "old":false
            }"#,
            EvaluatorResponseSchemaScope::AutoRestricted,
            "q",
            &["old".to_string()],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            EvaluatorResponseParseError::ShortIdResponse(_)
        ));
        assert!(error
            .to_string()
            .contains("already answered short ID `old`"));
    }

    #[test] // xpec: qv
    fn evaluator_response_rejects_unrequested_short_id_before_parsing_its_payload() {
        let error = parse_evaluator_response_for_short_id(
            r#"{
                "q":{"answer":"yes","evidence":"`src/main.rs:1`","qScopeSuggestion":["."]},
                "other":false
            }"#,
            EvaluatorResponseSchemaScope::AutoRestricted,
            "q",
            &[],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            EvaluatorResponseParseError::ShortIdResponse(_)
        ));
        assert!(error.to_string().contains("unrequested short ID `other`"));
    }

    #[test] // xpec: qv
    fn evaluator_response_rejects_invalid_short_id_as_a_mismatch() {
        let error = parse_evaluator_response_for_short_id(
            r#"{
                "q":{"answer":"yes","evidence":"`src/main.rs:1`","qScopeSuggestion":["."]},
                "bad-id":false
            }"#,
            EvaluatorResponseSchemaScope::AutoRestricted,
            "q",
            &[],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            EvaluatorResponseParseError::ShortIdResponse(_)
        ));
        assert!(error.to_string().contains("bad-id"));
    }

    #[test] // xpec: T5,qv
    fn evaluator_response_rejects_duplicate_result_field() {
        let error = parse_keyed_response_json(
            r#"{"answer":"yes","answer":"no","evidence":"duplicate","qScopeSuggestion":["."]}"#,
        )
        .unwrap_err();

        assert!(matches!(error, EvaluatorResponseParseError::Schema(_)));
    }

    #[test] // xpec: T5,qv
    fn evaluator_response_rejects_duplicate_short_id() {
        let error = parse_evaluator_response_json_for_exact_requested_short_ids(
            r#"{
                "q":false,
                "q":true
            }"#,
            &["q"],
            &[],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            EvaluatorResponseParseError::ShortIdResponse(_)
        ));
        assert!(error
            .to_string()
            .contains("duplicate evaluator response short ID `q`"));
    }

    #[test] // xpec: qv
    fn evaluator_response_output_schema_supports_each_requested_short_id() {
        let schema = evaluator_response_output_schema_for_exact_requested_short_ids(
            EvaluatorResponseSchemaScope::AutoRestricted,
            &["a", "b"],
        );

        assert_eq!(schema["required"], json!(["a", "b"]));
        assert!(schema["properties"].get("a").is_some());
        assert!(schema["properties"].get("b").is_some());
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test] // xpec: qv
    fn evaluator_response_parser_accepts_each_requested_short_id() {
        let responses = parse_evaluator_response_json_for_exact_requested_short_ids(
            r#"{
                "a":{"answer":"yes","evidence":"`src/main.rs:1`","qScopeSuggestion":["."]},
                "b":{"answer":"no","evidence":"`src/lib.rs:1`","qScopeSuggestion":["."]}
            }"#,
            &["a", "b"],
            &[],
        )
        .unwrap();

        assert_eq!(responses["a"].unvalidated_answer.as_deref(), Some("yes"));
        assert_eq!(responses["b"].unvalidated_answer.as_deref(), Some("no"));
        // xpec: 1g
        assert_eq!(responses["a"].evidence.as_deref(), Some("`src/main.rs:1`"));
        // xpec: 1g
        assert_eq!(responses["b"].evidence.as_deref(), Some("`src/lib.rs:1`"));
    }
}
