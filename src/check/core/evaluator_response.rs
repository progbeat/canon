use serde::{de, Deserialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

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
    pub(crate) evidence: String,
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
            evidence,
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
            evidence,
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

    #[cfg(test)]
    fn contains(&self, value: &str) -> bool {
        self.to_string().contains(value)
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

#[cfg(test)]
fn evaluator_response_json_schema(schema_scope: EvaluatorResponseSchemaScope) -> Value {
    json!({
        "type": "object",
        "propertyNames": {
            "pattern": "^[A-Za-z0-9]+$",
        },
        "additionalProperties": evaluator_response_result_json_schema(schema_scope),
    })
}

fn evaluator_response_result_json_schema(schema_scope: EvaluatorResponseSchemaScope) -> Value {
    let mut schema = json!({
        "type": "object",
        "properties": {
            "answer": {
                "type": "string",
                "pattern": ANSWER_PATTERN,
            },
            "error": {
                "type": "string",
                "enum": schema_scope.error_enum(),
            },
            "evidence": {
                "type": "string",
            },
        },
        "required": ["evidence"],
        "oneOf": [
            {"required": ["answer"], "not": { "required": ["error"] }},
            {"required": ["error"], "not": { "required": ["answer"] }},
        ],
        "additionalProperties": false,
    });
    if schema_scope.allows_question_scope_suggestion() {
        // Interrogations that may hide files outside the visible scope require
        // qScopeSuggestion, even when the q-scope is full project before
        // configured ignore exclusions. A full-scope first turn can still
        // propose a narrower reusable q-scope. Check modes that never hide
        // files use WithoutQuestionScopeSuggestion.
        schema["properties"]["qScopeSuggestion"] = json!({
            "type": "array",
            "minItems": 1,
            "items": {
                "type": "string",
                "minLength": 1,
                "pattern": "^[^\\r\\n]*$",
            },
        });
        if schema_scope.requires_question_scope_suggestion() {
            schema["required"] = json!(["evidence", "qScopeSuggestion"]);
        }
    }
    schema
}

pub(crate) fn evaluator_response_output_schema_for_scope(
    schema_scope: EvaluatorResponseSchemaScope,
    short_id: &str,
) -> Value {
    evaluator_response_output_schema_for_requested_short_ids(schema_scope, &[short_id])
}

fn evaluator_response_output_schema_for_requested_short_ids(
    schema_scope: EvaluatorResponseSchemaScope,
    short_ids: &[&str],
) -> Value {
    assert!(!short_ids.is_empty(), "requested short IDs are required");
    assert!(
        short_ids
            .iter()
            .all(|short_id| matches_short_id_pattern(short_id)),
        "requested short IDs must match the evaluator response pattern"
    );
    let result_schema = evaluator_response_output_result_json_schema(schema_scope);
    let required = short_ids.to_vec();
    let mut properties = serde_json::Map::new();
    for short_id in short_ids {
        properties.insert((*short_id).to_string(), result_schema.clone());
    }
    json!({
        "type": "object",
        "properties": Value::Object(properties),
        "required": required,
        "additionalProperties": false,
    })
}

fn evaluator_response_output_result_json_schema(
    schema_scope: EvaluatorResponseSchemaScope,
) -> Value {
    let schema = evaluator_response_result_json_schema(schema_scope);
    let properties = schema["properties"]
        .as_object()
        .expect("evaluator response schema has properties");
    // The app-server structured-output subset rejects `oneOf`. Two strict
    // `anyOf` branches preserve the same accepted shape without null transport
    // placeholders: one branch has `answer`, the other has `error`.
    json!({
        "anyOf": [
            evaluator_response_output_branch_schema(properties, schema_scope, "answer"),
            evaluator_response_output_branch_schema(properties, schema_scope, "error"),
        ],
    })
}

fn evaluator_response_output_branch_schema(
    properties: &serde_json::Map<String, Value>,
    schema_scope: EvaluatorResponseSchemaScope,
    result_field: &str,
) -> Value {
    let mut branch_properties = serde_json::Map::new();
    for key in ["evidence", result_field] {
        branch_properties.insert(
            key.to_string(),
            properties
                .get(key)
                .expect("evaluator response schema property exists")
                .clone(),
        );
    }
    let mut required = vec!["evidence", result_field];
    if schema_scope.requires_question_scope_suggestion() {
        branch_properties.insert(
            "qScopeSuggestion".to_string(),
            properties
                .get("qScopeSuggestion")
                .expect("evaluator response schema property exists")
                .clone(),
        );
        required.push("qScopeSuggestion");
    }
    json!({
        "type": "object",
        "properties": branch_properties,
        "required": required,
        "additionalProperties": false,
    })
}

#[cfg(test)]
fn evaluator_response_output_schema_for_schema_scope(
    schema_scope: EvaluatorResponseSchemaScope,
) -> Value {
    evaluator_response_output_schema_for_scope(schema_scope, "q")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluatorResponseJson {
    #[serde(default, deserialize_with = "deserialize_optional_answer")]
    pub(crate) answer: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_error")]
    pub(crate) error: Option<String>,
    pub(crate) evidence: String,
    #[serde(
        default,
        rename = "qScopeSuggestion",
        deserialize_with = "deserialize_optional_question_scope_suggestion"
    )]
    pub(crate) question_scope_suggestion: Option<Vec<String>>,
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
                self.evidence,
                question_scope_suggestion,
            ));
        }
        let error = self
            .error
            .expect("schema validation ensures error is present");
        // Restricted-scope schema permits qScopeSuggestion on an error response
        // too. Preserve it for schema fidelity and diagnostics; narrowing
        // policy still consumes suggestions only from answer responses.
        Ok(ParsedAnswer::error_with_question_scope_suggestion(
            error,
            self.evidence,
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

fn parse_evaluator_response_json_for_requested_short_ids(
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
    let mut raw = serde_json::from_str::<Value>(payload).map_err(|err| {
        EvaluatorResponseParseError::Schema(format!(
            "failed to parse evaluator JSON response: {}",
            err
        ))
    })?;
    let object = raw.as_object_mut().ok_or_else(|| {
        EvaluatorResponseParseError::Schema("evaluator response must be a JSON object".to_string())
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
        let response =
            serde_json::from_value::<EvaluatorResponseJson>(response).map_err(|err| {
                EvaluatorResponseParseError::Schema(format!(
                    "failed to parse evaluator JSON response for short ID `{}`: {}",
                    short_id, err
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
    if trimmed.is_empty() {
        return Err(EvaluatorResponseParseError::Schema(
            "evaluator response must be a JSON object".to_string(),
        ));
    }
    let mut deserializer = serde_json::Deserializer::from_str(trimmed);
    serde_json::Value::deserialize(&mut deserializer).map_err(|err| {
        EvaluatorResponseParseError::Schema(format!(
            "failed to inspect evaluator JSON response: {}",
            err
        ))
    })?;
    deserializer.end().map_err(|_| {
        EvaluatorResponseParseError::Schema(
            "evaluator response must not contain surrounding prose".to_string(),
        )
    })?;
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

#[cfg(test)]
mod tests {
    use super::{
        evaluator_response_json_schema, evaluator_response_output_schema_for_schema_scope,
        EvaluatorResponseJson, EvaluatorResponseParseError, EvaluatorResponseSchemaScope,
        ANSWER_PATTERN, ERROR_INVALID_QUESTION, ERROR_SCOPE_TOO_NARROW,
    };
    use serde_json::json;

    fn parse_evaluator_response(
        result_json: &str,
        schema_scope: EvaluatorResponseSchemaScope,
    ) -> Result<super::ParsedAnswer, EvaluatorResponseParseError> {
        super::parse_evaluator_response_for_short_id(
            &keyed_response(result_json),
            schema_scope,
            "q",
            &[],
        )
    }

    fn parse_evaluator_response_json(
        result_json: &str,
    ) -> Result<EvaluatorResponseJson, EvaluatorResponseParseError> {
        super::parse_evaluator_response_json(&keyed_response(result_json), "q", &[])
    }

    fn keyed_response(result_json: &str) -> String {
        format!(r#"{{"q":{}}}"#, result_json)
    }

    #[test]
    fn evaluator_response_requires_the_requested_short_id() {
        let error = super::parse_evaluator_response_for_short_id(
            &keyed_response(
                r#"{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":["."]}"#,
            ),
            EvaluatorResponseSchemaScope::Restricted,
            "other",
            &[],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            EvaluatorResponseParseError::ShortIdResponse(_)
        ));
        assert!(error.contains("other"));
    }

    #[test]
    fn evaluator_response_rejects_already_answered_short_id() {
        let error = super::parse_evaluator_response_for_short_id(
            &keyed_response(
                r#"{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":["."]}"#,
            ),
            EvaluatorResponseSchemaScope::Restricted,
            "q",
            &["q".to_string()],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            EvaluatorResponseParseError::ShortIdResponse(_)
        ));
        assert!(error.contains("already answered"));
    }

    #[test]
    fn evaluator_response_rejects_unrequested_short_id() {
        let error = super::parse_evaluator_response_for_short_id(
            r#"{
                "q":{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":["."]},
                "other":{"answer":"yes","evidence":"`src/lib.rs`","qScopeSuggestion":["."]}
            }"#,
            EvaluatorResponseSchemaScope::Restricted,
            "q",
            &[],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            EvaluatorResponseParseError::ShortIdResponse(_)
        ));
        assert!(error.contains("unrequested short ID `other`"));
    }

    #[test]
    fn evaluator_response_output_schema_supports_each_requested_short_id() {
        let schema = super::evaluator_response_output_schema_for_requested_short_ids(
            EvaluatorResponseSchemaScope::Restricted,
            &["a", "b"],
        );

        assert_eq!(schema["required"], json!(["a", "b"]));
        assert!(schema["properties"].get("a").is_some());
        assert!(schema["properties"].get("b").is_some());
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test]
    fn evaluator_response_parser_accepts_each_requested_short_id() {
        let responses = super::parse_evaluator_response_json_for_requested_short_ids(
            r#"{
                "a":{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":["."]},
                "b":{"answer":"no","evidence":"`src/lib.rs`","qScopeSuggestion":["."]}
            }"#,
            &["a", "b"],
            &[],
        )
        .unwrap();

        assert_eq!(responses["a"].answer.as_deref(), Some("yes"));
        assert_eq!(responses["b"].answer.as_deref(), Some("no"));
    }

    #[test]
    fn restricted_evaluator_response_json_schema_matches_interrogation_policy() {
        let schema = evaluator_response_json_schema(EvaluatorResponseSchemaScope::Restricted);
        let result_schema = &schema["additionalProperties"];

        assert_eq!(schema["propertyNames"]["pattern"], json!("^[A-Za-z0-9]+$"));
        assert_eq!(
            result_schema["required"],
            json!(["evidence", "qScopeSuggestion"])
        );
        assert_eq!(
            result_schema["properties"]["answer"]["type"],
            json!("string")
        );
        assert_eq!(
            result_schema["properties"]["answer"]["pattern"],
            json!(ANSWER_PATTERN)
        );
        assert_eq!(
            result_schema["properties"]["error"]["type"],
            json!("string")
        );
        assert_eq!(
            result_schema["properties"]["error"]["enum"],
            json!([ERROR_SCOPE_TOO_NARROW, ERROR_INVALID_QUESTION])
        );
        assert_eq!(result_schema["oneOf"][0]["required"], json!(["answer"]));
        assert_eq!(result_schema["oneOf"][1]["required"], json!(["error"]));
        assert_eq!(result_schema["additionalProperties"], json!(false));
    }

    #[test]
    fn full_project_evaluator_response_json_schema_disables_scope_too_narrow() {
        let schema = evaluator_response_json_schema(EvaluatorResponseSchemaScope::FullProject);
        let result_schema = &schema["additionalProperties"];

        assert_eq!(
            result_schema["required"],
            json!(["evidence", "qScopeSuggestion"])
        );
        assert!(result_schema["properties"]
            .get("qScopeSuggestion")
            .is_some());
        assert_eq!(
            result_schema["properties"]["error"]["enum"],
            json!([ERROR_INVALID_QUESTION])
        );
    }

    #[test]
    fn without_question_scope_suggestion_evaluator_response_json_schema_omits_question_scope_suggestion(
    ) {
        let schema = evaluator_response_json_schema(
            EvaluatorResponseSchemaScope::WithoutQuestionScopeSuggestion,
        );
        let result_schema = &schema["additionalProperties"];

        assert_eq!(result_schema["required"], json!(["evidence"]));
        assert!(result_schema["properties"]
            .get("qScopeSuggestion")
            .is_none());
        assert_eq!(
            result_schema["properties"]["error"]["enum"],
            json!([ERROR_INVALID_QUESTION])
        );
    }

    #[test]
    fn restricted_evaluator_response_output_schema_matches_interrogation_policy() {
        let schema = evaluator_response_output_schema_for_schema_scope(
            EvaluatorResponseSchemaScope::Restricted,
        );
        let result_schema = &schema["properties"]["q"];

        assert_eq!(schema["required"], json!(["q"]));
        assert_eq!(schema["additionalProperties"], json!(false));
        let answer_branch = &result_schema["anyOf"][0];
        let error_branch = &result_schema["anyOf"][1];
        assert_eq!(
            error_branch["properties"]["error"]["enum"],
            json!([ERROR_SCOPE_TOO_NARROW, ERROR_INVALID_QUESTION])
        );
        assert!(result_schema.get("oneOf").is_none());
        assert_eq!(
            answer_branch["required"],
            json!(["evidence", "answer", "qScopeSuggestion"])
        );
        assert!(answer_branch["properties"].get("error").is_none());
        assert_eq!(
            error_branch["required"],
            json!(["evidence", "error", "qScopeSuggestion"])
        );
        assert!(error_branch["properties"].get("answer").is_none());
    }

    #[test]
    fn full_project_evaluator_response_output_schema_disables_scope_too_narrow() {
        let schema = evaluator_response_output_schema_for_schema_scope(
            EvaluatorResponseSchemaScope::FullProject,
        );
        let result_schema = &schema["properties"]["q"];

        let answer_branch = &result_schema["anyOf"][0];
        let error_branch = &result_schema["anyOf"][1];
        assert_eq!(
            error_branch["properties"]["error"]["enum"],
            json!([ERROR_INVALID_QUESTION])
        );
        assert!(result_schema.get("oneOf").is_none());
        assert_eq!(
            answer_branch["required"],
            json!(["evidence", "answer", "qScopeSuggestion"])
        );
        assert_eq!(
            error_branch["required"],
            json!(["evidence", "error", "qScopeSuggestion"])
        );
    }

    #[test]
    fn without_question_scope_suggestion_evaluator_response_output_schema_omits_question_scope_suggestion(
    ) {
        let schema = evaluator_response_output_schema_for_schema_scope(
            EvaluatorResponseSchemaScope::WithoutQuestionScopeSuggestion,
        );
        let result_schema = &schema["properties"]["q"];

        let answer_branch = &result_schema["anyOf"][0];
        let error_branch = &result_schema["anyOf"][1];
        assert_eq!(
            error_branch["properties"]["error"]["enum"],
            json!([ERROR_INVALID_QUESTION])
        );
        assert!(result_schema.get("oneOf").is_none());
        assert_eq!(answer_branch["required"], json!(["evidence", "answer"]));
        assert!(answer_branch["properties"]
            .get("qScopeSuggestion")
            .is_none());
        assert_eq!(error_branch["required"], json!(["evidence", "error"]));
        assert!(error_branch["properties"].get("qScopeSuggestion").is_none());
    }

    #[test]
    fn full_project_evaluator_response_rejects_scope_too_narrow() {
        let error = parse_evaluator_response(
            r#"{"error":"ScopeTooNarrow","evidence":"scope"}"#,
            EvaluatorResponseSchemaScope::FullProject,
        )
        .unwrap_err();

        assert!(error.contains("ScopeTooNarrow"));
    }

    #[test]
    fn restricted_evaluator_response_requires_question_scope_suggestion() {
        let response =
            parse_evaluator_response_json(r#"{"answer":"yes","evidence":"`src/main.rs`"}"#)
                .unwrap();
        let error = response
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .unwrap_err();

        assert!(error.contains("qScopeSuggestion"));
    }

    #[test]
    fn full_project_evaluator_response_requires_question_scope_suggestion() {
        let response =
            parse_evaluator_response_json(r#"{"answer":"yes","evidence":"`src/main.rs`"}"#)
                .unwrap();
        let error = response
            .validate_schema(EvaluatorResponseSchemaScope::FullProject)
            .unwrap_err();

        assert!(error.contains("qScopeSuggestion"));
    }

    #[test]
    fn evaluator_response_rejects_null_fields() {
        let error = parse_evaluator_response(
            r#"{"answer":"yes","error":null,"evidence":"`src/main.rs`","qScopeSuggestion":null}"#,
            EvaluatorResponseSchemaScope::FullProject,
        )
        .unwrap_err();

        assert!(error.contains("must not be null"));
    }

    #[test]
    fn without_question_scope_suggestion_evaluator_response_omits_question_scope_suggestion() {
        let response = parse_evaluator_response(
            r#"{"answer":"yes","evidence":"`src/main.rs`"}"#,
            EvaluatorResponseSchemaScope::WithoutQuestionScopeSuggestion,
        )
        .unwrap();

        assert_eq!(response.observed, "yes");
        assert_eq!(response.question_scope_suggestion, None);
    }

    #[test]
    fn full_project_evaluator_response_accepts_question_scope_suggestion() {
        let response = parse_evaluator_response(
            r#"{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":["src/main.rs"]}"#,
            EvaluatorResponseSchemaScope::FullProject,
        )
        .unwrap();

        assert_eq!(
            response.question_scope_suggestion,
            Some(vec!["src/main.rs".to_string()])
        );
    }

    #[test]
    fn without_question_scope_suggestion_evaluator_response_rejects_question_scope_suggestion() {
        let error = parse_evaluator_response(
            r#"{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":["."]}"#,
            EvaluatorResponseSchemaScope::WithoutQuestionScopeSuggestion,
        )
        .unwrap_err();

        assert!(error.contains("qScopeSuggestion"));
    }

    #[test]
    fn evaluator_response_rejects_empty_question_scope_suggestion() {
        let response = parse_evaluator_response_json(
            r#"{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":[]}"#,
        )
        .unwrap();

        assert!(response
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .unwrap_err()
            .contains("qScopeSuggestion"));
    }

    #[test]
    fn evaluator_response_rejects_empty_question_scope_suggestion_item() {
        let response = parse_evaluator_response_json(
            r#"{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":[""]}"#,
        )
        .unwrap();

        assert!(response
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .unwrap_err()
            .contains("qScopeSuggestion"));
    }

    #[test]
    fn evaluator_response_accepts_required_question_scope_suggestion() {
        let response = parse_evaluator_response_json(
            r#"{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":["src/main.rs"]}"#,
        )
        .unwrap();

        response
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .unwrap();
        assert_eq!(
            response.question_scope_suggestion,
            Some(vec!["src/main.rs".to_string()])
        );
    }

    #[test]
    fn evaluator_response_schema_allows_non_crlf_question_scope_chars() {
        let response = parse_evaluator_response_json(
            "{\"answer\":\"yes\",\"evidence\":\"`src/main.rs`\",\"qScopeSuggestion\":[\"src/main.rs\\u0008\"]}",
        )
        .unwrap();

        response
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .unwrap();
    }

    #[test]
    fn evaluator_response_schema_rejects_answers_outside_answer_pattern() {
        for invalid_answer in ["Rust", "yes\t", "yes\n", "yes\u{2028}still", ""] {
            let response = EvaluatorResponseJson {
                answer: Some(invalid_answer.to_string()),
                error: None,
                evidence: "ok".to_string(),
                question_scope_suggestion: Some(vec![".".to_string()]),
            };

            assert!(response
                .validate_schema(EvaluatorResponseSchemaScope::Restricted)
                .unwrap_err()
                .contains("answer"));
        }
    }

    #[test]
    fn evaluator_response_schema_rejects_non_canonical_error_token() {
        let response = EvaluatorResponseJson {
            answer: None,
            error: Some("TechnicalFailure".to_string()),
            evidence: "ok".to_string(),
            question_scope_suggestion: Some(vec![".".to_string()]),
        };

        assert!(response
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .unwrap_err()
            .contains("unsupported evaluator error"));
    }

    #[test]
    fn evaluator_response_schema_rejects_crlf_in_single_line_fields() {
        let answer = parse_evaluator_response_json(
            "{\"answer\":\"yes\\n\",\"evidence\":\"`src/main.rs`\",\"qScopeSuggestion\":[\"src/main.rs\"]}",
        )
        .unwrap();
        let q_scope = parse_evaluator_response_json(
            "{\"answer\":\"yes\",\"evidence\":\"`src/main.rs`\",\"qScopeSuggestion\":[\"src\\rmain.rs\"]}",
        )
        .unwrap();

        assert!(answer
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .unwrap_err()
            .contains("answer"));
        assert!(q_scope
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .unwrap_err()
            .contains("qScopeSuggestion"));
    }

    #[test]
    fn evaluator_response_schema_rejects_only_crlf_q_scope_line_breaks() {
        let schema_valid_unicode_separator = EvaluatorResponseJson {
            answer: Some("yes".to_string()),
            error: None,
            evidence: "ok".to_string(),
            question_scope_suggestion: Some(vec!["src\u{2028}main.rs".to_string()]),
        };
        assert!(schema_valid_unicode_separator
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .is_ok());

        let schema_invalid_crlf = EvaluatorResponseJson {
            answer: Some("yes".to_string()),
            error: None,
            evidence: "ok".to_string(),
            question_scope_suggestion: Some(vec!["src\nmain.rs".to_string()]),
        };
        assert!(schema_invalid_crlf
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .is_err());
    }
}
