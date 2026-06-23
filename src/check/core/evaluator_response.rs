use serde::{de, Deserialize};
use serde_json::{json, Value};

pub(crate) const ERROR_SCOPE_TOO_NARROW: &str = "ScopeTooNarrow";
pub(crate) const ERROR_INVALID_QUESTION: &str = "InvalidQuestion";
pub(crate) const ANSWER_PATTERN: &str = "^[-_a-z0-9]+$";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum EvaluatorResponseSchemaScope {
    Restricted,
    FullProject,
}

impl EvaluatorResponseSchemaScope {
    pub(crate) fn for_q_scope(q_scope: &[String]) -> EvaluatorResponseSchemaScope {
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
            // Full-project-scope interrogations disable ScopeTooNarrow.
            EvaluatorResponseSchemaScope::FullProject => json!([ERROR_INVALID_QUESTION]),
        }
    }

    fn allows_error(self, error: &str) -> bool {
        match self {
            EvaluatorResponseSchemaScope::Restricted => {
                matches!(error, ERROR_SCOPE_TOO_NARROW | ERROR_INVALID_QUESTION)
            }
            EvaluatorResponseSchemaScope::FullProject => error == ERROR_INVALID_QUESTION,
        }
    }

    fn requires_question_scope_suggestion(self) -> bool {
        matches!(self, EvaluatorResponseSchemaScope::Restricted)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedAnswer {
    pub(crate) answer: String,
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
            answer,
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
            answer: error.clone(),
            error: Some(error),
            evidence,
            scope: Vec::new(),
            question_scope_suggestion,
        }
    }
}

pub(crate) fn parse_evaluator_response(
    text: &str,
    schema_scope: EvaluatorResponseSchemaScope,
) -> Result<ParsedAnswer, String> {
    let response = parse_evaluator_response_json(text)?;
    response.into_schema_valid_parsed_answer(schema_scope)
}

pub(crate) fn evaluator_response_json_schema(schema_scope: EvaluatorResponseSchemaScope) -> Value {
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
    if schema_scope.requires_question_scope_suggestion() {
        // Interrogation Policy's restricted-scope schema requires
        // qScopeSuggestion. Full-project schema takes the opposite branch and
        // omits the property entirely because nothing narrower is needed.
        schema["properties"]["qScopeSuggestion"] = json!({
            "type": "array",
            "minItems": 1,
            "items": {
                "type": "string",
                "minLength": 1,
                "pattern": "^[^\\r\\n]*$",
            },
        });
        schema["required"] = json!(["evidence", "qScopeSuggestion"]);
    }
    schema
}

pub(crate) fn evaluator_response_output_schema_for_q_scope(q_scope: &[String]) -> Value {
    // Interrogation Policy navigation: this module only builds and validates
    // the response schema for one evaluator turn. Follow-up sequencing for
    // check runs lives in `src/check/run/execute/expectation.rs`; query-mode
    // sequencing lives in `src/check/interrogation/query/mod.rs`; the
    // q-scope verification gate and acceptance matrix live in
    // `src/check/interrogation/policy.rs`; model retry order and thinking
    // selection flow through `src/check/interrogation/session/model_fallback.rs`,
    // `src/check/interrogation/session/thread.rs`, and
    // `src/check/interrogation/state.rs`.
    match EvaluatorResponseSchemaScope::for_q_scope(q_scope) {
        EvaluatorResponseSchemaScope::Restricted => restricted_evaluator_response_output_schema(),
        EvaluatorResponseSchemaScope::FullProject => {
            full_project_evaluator_response_output_schema()
        }
    }
}

#[cfg(test)]
fn evaluator_response_output_schema_for_schema_scope(
    schema_scope: EvaluatorResponseSchemaScope,
) -> Value {
    match schema_scope {
        EvaluatorResponseSchemaScope::Restricted => restricted_evaluator_response_output_schema(),
        EvaluatorResponseSchemaScope::FullProject => {
            full_project_evaluator_response_output_schema()
        }
    }
}

fn restricted_evaluator_response_output_schema() -> Value {
    evaluator_response_output_schema_with_error_enum(
        EvaluatorResponseSchemaScope::Restricted,
        json!([ERROR_SCOPE_TOO_NARROW, ERROR_INVALID_QUESTION, null]),
    )
}

fn full_project_evaluator_response_output_schema() -> Value {
    // This value is sent as turn/start.params.outputSchema for q-scope ["."],
    // so the app server cannot return ScopeTooNarrow as a schema-valid
    // full-project-scope response.
    evaluator_response_output_schema_with_error_enum(
        EvaluatorResponseSchemaScope::FullProject,
        json!([ERROR_INVALID_QUESTION, null]),
    )
}

fn evaluator_response_output_schema_with_error_enum(
    schema_scope: EvaluatorResponseSchemaScope,
    output_error_enum: Value,
) -> Value {
    // Codex app-server structured output requires every object property to be
    // listed in `required` and represents optional fields as nullable types.
    // Parsing removes those null placeholders; `validate_schema` then applies
    // the Interrogation Policy's canonical exactly-one validation.
    let mut schema = evaluator_response_json_schema(schema_scope);
    let object = schema
        .as_object_mut()
        .expect("evaluator response schema is an object");
    object.insert(
        "required".to_string(),
        if schema_scope.requires_question_scope_suggestion() {
            json!(["answer", "error", "evidence", "qScopeSuggestion"])
        } else {
            json!(["answer", "error", "evidence"])
        },
    );
    object.remove("oneOf");
    let properties = object
        .get_mut("properties")
        .and_then(Value::as_object_mut)
        .expect("evaluator response schema has properties");
    properties
        .get_mut("answer")
        .expect("evaluator response schema has answer")["type"] = json!(["string", "null"]);
    let error = properties
        .get_mut("error")
        .expect("evaluator response schema has error");
    error["type"] = json!(["string", "null"]);
    error["enum"] = output_error_enum;
    schema
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
        match (schema_scope, self.question_scope_suggestion.as_ref()) {
            (EvaluatorResponseSchemaScope::Restricted, Some(items)) => {
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
            (EvaluatorResponseSchemaScope::Restricted, None) => {
                return Err("qScopeSuggestion is required".to_string());
            }
            (EvaluatorResponseSchemaScope::FullProject, Some(_)) => {
                return Err("qScopeSuggestion must be omitted for full project scope".to_string());
            }
            (EvaluatorResponseSchemaScope::FullProject, None) => {}
        }
        Ok(())
    }
}

pub(crate) fn parse_evaluator_response_json(text: &str) -> Result<EvaluatorResponseJson, String> {
    let payload = evaluator_response_json_payload(text)?;
    let mut raw = serde_json::from_str::<Value>(payload)
        .map_err(|err| format!("failed to parse evaluator JSON response: {}", err))?;
    normalize_output_schema_null_placeholders(&mut raw)?;
    serde_json::from_value::<EvaluatorResponseJson>(raw)
        .map_err(|err| format!("failed to parse evaluator JSON response: {}", err))
}

fn normalize_output_schema_null_placeholders(raw: &mut Value) -> Result<(), String> {
    let Some(object) = raw.as_object_mut() else {
        return Err("evaluator response must be a JSON object".to_string());
    };
    if object.get("qScopeSuggestion").is_some_and(Value::is_null) {
        return Err("qScopeSuggestion must not be null".to_string());
    }
    for key in ["answer", "error"] {
        if object.get(key).is_some_and(Value::is_null) {
            object.remove(key);
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
        parse_evaluator_response, parse_evaluator_response_json, EvaluatorResponseJson,
        EvaluatorResponseSchemaScope, ANSWER_PATTERN, ERROR_INVALID_QUESTION,
        ERROR_SCOPE_TOO_NARROW,
    };
    use serde_json::json;

    #[test]
    fn restricted_evaluator_response_json_schema_matches_interrogation_policy() {
        let schema = evaluator_response_json_schema(EvaluatorResponseSchemaScope::Restricted);

        assert_eq!(schema["required"], json!(["evidence", "qScopeSuggestion"]));
        assert_eq!(schema["properties"]["answer"]["type"], json!("string"));
        assert_eq!(
            schema["properties"]["answer"]["pattern"],
            json!(ANSWER_PATTERN)
        );
        assert_eq!(schema["properties"]["error"]["type"], json!("string"));
        assert_eq!(
            schema["properties"]["error"]["enum"],
            json!([ERROR_SCOPE_TOO_NARROW, ERROR_INVALID_QUESTION])
        );
        assert_eq!(schema["oneOf"][0]["required"], json!(["answer"]));
        assert_eq!(schema["oneOf"][1]["required"], json!(["error"]));
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test]
    fn full_project_evaluator_response_json_schema_disables_scope_too_narrow() {
        let schema = evaluator_response_json_schema(EvaluatorResponseSchemaScope::FullProject);

        assert_eq!(schema["required"], json!(["evidence"]));
        assert!(schema["properties"].get("qScopeSuggestion").is_none());
        assert_eq!(
            schema["properties"]["error"]["enum"],
            json!([ERROR_INVALID_QUESTION])
        );
    }

    #[test]
    fn restricted_evaluator_response_output_schema_uses_app_server_dialect() {
        let schema = evaluator_response_output_schema_for_schema_scope(
            EvaluatorResponseSchemaScope::Restricted,
        );

        assert_eq!(
            schema["required"],
            json!(["answer", "error", "evidence", "qScopeSuggestion"])
        );
        assert_eq!(
            schema["properties"]["answer"]["type"],
            json!(["string", "null"])
        );
        assert_eq!(
            schema["properties"]["error"]["type"],
            json!(["string", "null"])
        );
        assert_eq!(
            schema["properties"]["error"]["enum"],
            json!([ERROR_SCOPE_TOO_NARROW, ERROR_INVALID_QUESTION, null])
        );
        assert!(schema.get("oneOf").is_none());
    }

    #[test]
    fn full_project_evaluator_response_output_schema_disables_scope_too_narrow() {
        let schema = evaluator_response_output_schema_for_schema_scope(
            EvaluatorResponseSchemaScope::FullProject,
        );

        assert_eq!(schema["required"], json!(["answer", "error", "evidence"]));
        assert!(schema["properties"].get("qScopeSuggestion").is_none());
        assert_eq!(
            schema["properties"]["error"]["enum"],
            json!([ERROR_INVALID_QUESTION, null])
        );
        assert!(schema.get("oneOf").is_none());
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
    fn full_project_evaluator_response_omits_question_scope_suggestion() {
        let response = parse_evaluator_response(
            r#"{"answer":"yes","evidence":"`src/main.rs`"}"#,
            EvaluatorResponseSchemaScope::FullProject,
        )
        .unwrap();

        assert_eq!(response.answer, "yes");
        assert_eq!(response.question_scope_suggestion, None);
    }

    #[test]
    fn full_project_evaluator_response_rejects_question_scope_suggestion() {
        let error = parse_evaluator_response(
            r#"{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":["."]}"#,
            EvaluatorResponseSchemaScope::FullProject,
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
    fn evaluator_response_treats_output_schema_null_placeholders_as_absent() {
        let response = parse_evaluator_response_json(
            r#"{"answer":"yes","error":null,"evidence":"`src/main.rs`","qScopeSuggestion":["src/main.rs"]}"#,
        )
        .unwrap();

        response
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .unwrap();
        assert_eq!(response.answer.as_deref(), Some("yes"));
        assert_eq!(response.error, None);
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
