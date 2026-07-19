use serde::{de, Deserialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

mod contract;

pub(crate) use contract::evaluator_response_output_schema_for_scope;
#[cfg(test)]
pub(crate) use contract::{
    evaluator_response_json_schema, evaluator_response_output_schema_for_requested_short_ids,
    evaluator_response_output_schema_for_schema_scope,
};

pub(crate) const ERROR_SCOPE_TOO_NARROW: &str = "ScopeTooNarrow";
pub(crate) const ERROR_INVALID_QUESTION: &str = "InvalidQuestion";
pub(crate) const ANSWER_PATTERN: &str = "^[-_a-z0-9]+$";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum EvaluatorResponseSchemaScope {
    Restricted,
    // Git-backed full-project interrogations keep qScopeSuggestion so a
    // passing full-scope response can seed a narrower future q-scope; they
    // differ from restricted scope by disabling ScopeTooNarrow.
    FullProject,
    // Used only when the check mode never hides files from the evaluator.
    WithoutQuestionScopeSuggestion,
}

impl EvaluatorResponseSchemaScope {
    pub(crate) fn for_scope_with_question_scope_suggestion(
        q_scope: &[String],
    ) -> EvaluatorResponseSchemaScope {
        // Interrogation Policy defines full project scope as exactly q-scope
        // ["."], before configured ignore exclusions are applied.
        if q_scope.len() == 1 && q_scope[0] == "." {
            EvaluatorResponseSchemaScope::FullProject
        } else {
            EvaluatorResponseSchemaScope::Restricted
        }
    }

    fn error_enum(self) -> Value {
        // Evaluator response schemas only constrain evaluator-produced result
        // errors. Final `canon check` Error lines render CheckRecord errors,
        // which may also come from runtime or configuration failures.
        match self {
            // Restricted-scope interrogations may ask for more visible scope.
            EvaluatorResponseSchemaScope::Restricted => {
                json!([ERROR_SCOPE_TOO_NARROW, ERROR_INVALID_QUESTION])
            }
            // Full-project-scope and no-suggestion schemas disable ScopeTooNarrow.
            EvaluatorResponseSchemaScope::FullProject
            | EvaluatorResponseSchemaScope::WithoutQuestionScopeSuggestion => {
                json!([ERROR_INVALID_QUESTION])
            }
        }
    }

    fn allows_error(self, error: &str) -> bool {
        match self {
            EvaluatorResponseSchemaScope::Restricted => {
                matches!(error, ERROR_SCOPE_TOO_NARROW | ERROR_INVALID_QUESTION)
            }
            EvaluatorResponseSchemaScope::FullProject
            | EvaluatorResponseSchemaScope::WithoutQuestionScopeSuggestion => {
                error == ERROR_INVALID_QUESTION
            }
        }
    }

    fn requires_question_scope_suggestion(self) -> bool {
        // Interrogation Policy's restricted-scope schema requires
        // qScopeSuggestion. Full-project scope keeps that requirement and only
        // removes ScopeTooNarrow from the allowed error values.
        matches!(
            self,
            EvaluatorResponseSchemaScope::Restricted | EvaluatorResponseSchemaScope::FullProject
        )
    }

    fn allows_question_scope_suggestion(self) -> bool {
        !matches!(
            self,
            EvaluatorResponseSchemaScope::WithoutQuestionScopeSuggestion
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedAnswer {
    pub(crate) observed: String,
    pub(crate) error: Option<String>,
    pub(crate) evidence: Option<String>,
    pub(crate) scope: Vec<String>,
    pub(crate) question_scope_suggestion: Option<Vec<String>>,
}

impl ParsedAnswer {
    pub(crate) fn answer(
        answer: String,
        evidence: String,
        question_scope_suggestion: Option<Vec<String>>,
    ) -> ParsedAnswer {
        ParsedAnswer {
            observed: answer,
            error: None,
            evidence: Some(evidence),
            scope: Vec::new(),
            question_scope_suggestion,
        }
    }

    pub(crate) fn error(error: String, evidence: String) -> ParsedAnswer {
        ParsedAnswer::error_with_question_scope_suggestion(error, evidence, None)
    }

    pub(crate) fn error_with_question_scope_suggestion(
        error: String,
        evidence: String,
        question_scope_suggestion: Option<Vec<String>>,
    ) -> ParsedAnswer {
        ParsedAnswer {
            observed: error.clone(),
            error: Some(error),
            evidence: Some(evidence),
            scope: Vec::new(),
            question_scope_suggestion,
        }
    }

    fn evaluator_error(
        error: String,
        question_scope_suggestion: Option<Vec<String>>,
    ) -> ParsedAnswer {
        ParsedAnswer {
            observed: error.clone(),
            error: Some(error),
            evidence: None,
            scope: Vec::new(),
            question_scope_suggestion,
        }
    }
}

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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluatorResponseJson {
    #[serde(default, deserialize_with = "deserialize_optional_answer")]
    pub(crate) answer: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_error")]
    pub(crate) error: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_evidence")]
    pub(crate) evidence: Option<String>,
    #[serde(
        default,
        rename = "qScopeSuggestion",
        deserialize_with = "deserialize_optional_question_scope_suggestion"
    )]
    pub(crate) question_scope_suggestion: Option<Vec<String>>,
}

struct EvaluatorResponsesJson(BTreeMap<String, EvaluatorResponseJson>);

impl<'de> Deserialize<'de> for EvaluatorResponsesJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        struct ResponsesVisitor;

        impl<'de> de::Visitor<'de> for ResponsesVisitor {
            type Value = EvaluatorResponsesJson;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an evaluator response object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut responses = BTreeMap::new();
                while let Some(short_id) = map.next_key::<String>()? {
                    if responses.contains_key(&short_id) {
                        return Err(de::Error::custom(format!(
                            "duplicate evaluator response short ID `{}`",
                            short_id
                        )));
                    }
                    let response = map.next_value::<EvaluatorResponseJson>()?;
                    responses.insert(short_id, response);
                }
                Ok(EvaluatorResponsesJson(responses))
            }
        }

        deserializer.deserialize_map(ResponsesVisitor)
    }
}

impl EvaluatorResponseJson {
    fn into_schema_valid_parsed_answer(
        self,
        schema_scope: EvaluatorResponseSchemaScope,
    ) -> Result<ParsedAnswer, String> {
        self.validate_schema(schema_scope)?;
        let question_scope_suggestion = self.question_scope_suggestion;
        if let Some(answer) = self.answer {
            // Pass/fail comparison happens after parsing against the
            // expectation's current expected answer.
            return Ok(ParsedAnswer::answer(
                answer,
                self.evidence
                    .expect("schema validation ensures answer evidence is present"),
                question_scope_suggestion,
            ));
        }
        let error = self
            .error
            .expect("schema validation ensures error is present");
        // Restricted-scope schema permits qScopeSuggestion on an error response
        // too. Preserve it for schema fidelity and diagnostics; narrowing
        // policy still consumes suggestions only from answer responses.
        Ok(ParsedAnswer::evaluator_error(
            error,
            question_scope_suggestion,
        ))
    }

    pub(crate) fn validate_schema(
        &self,
        schema_scope: EvaluatorResponseSchemaScope,
    ) -> Result<(), String> {
        let has_answer = self.answer.is_some();
        let has_error = self.error.is_some();
        if has_answer == has_error {
            return Err(
                "evaluator response must contain exactly one of answer or error".to_string(),
            );
        }
        if has_answer && self.evidence.is_none() {
            return Err("evidence is required with answer".to_string());
        }
        if has_error && self.evidence.is_some() {
            return Err("evidence must be omitted with error".to_string());
        }
        if let Some(answer) = self.answer.as_deref() {
            if !matches_answer_pattern(answer) {
                return Err(format!("answer must match pattern {}", ANSWER_PATTERN));
            }
        }
        if let Some(error) = self.error.as_deref() {
            if !schema_scope.allows_error(error) {
                return Err(format!("unsupported evaluator error: {}", error));
            }
        }
        if let Some(items) = self.question_scope_suggestion.as_ref() {
            if !schema_scope.allows_question_scope_suggestion() {
                return Err("qScopeSuggestion must be omitted when no files are hidden".to_string());
            }
            {
                if items.is_empty() {
                    return Err("qScopeSuggestion must contain at least one item".to_string());
                }
                for item in items {
                    // Mirror the selected schema's minLength and CR/LF-free
                    // pattern checks that may be enforced after parsing when
                    // transport schema support is incomplete.
                    if item.is_empty() || contains_schema_single_line_violation(item) {
                        return Err(
                            "qScopeSuggestion items must be non-empty single-line strings"
                                .to_string(),
                        );
                    }
                }
            }
        } else if schema_scope.requires_question_scope_suggestion() {
            return Err("qScopeSuggestion is required".to_string());
        }
        Ok(())
    }
}

pub(crate) fn parse_evaluator_response_json(
    text: &str,
    short_id: &str,
    answered_short_ids: &[String],
) -> Result<EvaluatorResponseJson, EvaluatorResponseParseError> {
    let mut responses = parse_evaluator_response_json_for_requested_short_ids(
        text,
        &[short_id],
        answered_short_ids,
    )?;
    Ok(responses
        .remove(short_id)
        .expect("requested short ID was checked above"))
}

pub(crate) fn parse_evaluator_response_json_for_requested_short_ids(
    text: &str,
    requested_short_ids: &[&str],
    answered_short_ids: &[String],
) -> Result<BTreeMap<String, EvaluatorResponseJson>, EvaluatorResponseParseError> {
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
    // [T] Deserialize the outer map and each typed result in one streaming
    // pass. The map visitor observes duplicate short IDs, while the derived
    // result deserializer observes duplicate fields before either can be
    // collapsed into a generic JSON object.
    let mut deserializer = serde_json::Deserializer::from_str(payload);
    let EvaluatorResponsesJson(mut object) = EvaluatorResponsesJson::deserialize(&mut deserializer)
        .map_err(|err| {
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
    for key in object.keys() {
        if !matches_short_id_pattern(key) {
            return Err(EvaluatorResponseParseError::Schema(format!(
                "evaluator response short ID must match pattern ^[A-Za-z0-9]+$: {}",
                key
            )));
        }
    }
    let answered = answered_short_ids.iter().cloned().collect::<BTreeSet<_>>();
    for key in object.keys() {
        if answered.contains(key) {
            return Err(EvaluatorResponseParseError::ShortIdResponse(format!(
                "evaluator response returned already answered short ID `{}`",
                key
            )));
        }
    }
    for short_id in &requested {
        if !object.contains_key(short_id) {
            return Err(EvaluatorResponseParseError::ShortIdResponse(format!(
                "evaluator response did not contain short ID `{}`",
                short_id
            )));
        }
    }
    if let Some(key) = object.keys().find(|key| !requested.contains(key.as_str())) {
        return Err(EvaluatorResponseParseError::ShortIdResponse(format!(
            "evaluator response returned unrequested short ID `{}`",
            key
        )));
    }
    let mut responses = BTreeMap::new();
    for short_id in requested_short_ids {
        let response = object
            .remove(*short_id)
            .expect("requested short ID was checked above");
        responses.insert((*short_id).to_string(), response);
    }
    Ok(responses)
}

pub(crate) fn evaluator_response_json_payload(
    text: &str,
) -> Result<&str, EvaluatorResponseParseError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(EvaluatorResponseParseError::Schema(
            "evaluator response must be a JSON object".to_string(),
        ));
    }
    if !trimmed.starts_with('{') {
        return Err(EvaluatorResponseParseError::Schema(
            "evaluator response must be a JSON object".to_string(),
        ));
    }
    Ok(trimmed)
}

fn contains_schema_single_line_violation(value: &str) -> bool {
    // Interrogation Policy defines the evaluator JSON Schema pattern as
    // CR/LF-only. Other Unicode line separators remain schema-valid text and
    // are escaped later when rendered in check output.
    value.chars().any(|char| matches!(char, '\r' | '\n'))
}

pub(crate) fn matches_answer_pattern(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| matches!(byte, b'-' | b'_' | b'0'..=b'9' | b'a'..=b'z'))
}

fn matches_short_id_pattern(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn deserialize_optional_answer<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: de::Deserializer<'de>,
{
    deserialize_optional_string_field(deserializer, "answer")
}

fn deserialize_optional_error<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: de::Deserializer<'de>,
{
    deserialize_optional_string_field(deserializer, "error")
}

fn deserialize_optional_evidence<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: de::Deserializer<'de>,
{
    deserialize_optional_string_field(deserializer, "evidence")
}

fn deserialize_optional_string_field<'de, D>(
    deserializer: D,
    field_name: &'static str,
) -> Result<Option<String>, D::Error>
where
    D: de::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_null() {
        return Err(de::Error::custom(format!(
            "{} must not be null",
            field_name
        )));
    }
    String::deserialize(value)
        .map(Some)
        .map_err(de::Error::custom)
}

fn deserialize_optional_question_scope_suggestion<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: de::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_null() {
        return Err(de::Error::custom("qScopeSuggestion must not be null"));
    }
    Vec::<String>::deserialize(value)
        .map(Some)
        .map_err(de::Error::custom)
}
