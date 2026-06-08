use serde::{de, Deserialize};
use serde_json::{json, Value};

pub(crate) const ERROR_INSUFFICIENT_EVIDENCE: &str = "insufficient-evidence";
pub(crate) const ERROR_INVALID_QUESTION: &str = "invalid-question";
pub(crate) const ERROR_UNPARSABLE: &str = "unparsable";

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

pub(crate) fn parse_evaluator_response(text: &str) -> Result<ParsedAnswer, String> {
    let response = parse_evaluator_response_json(text)?;
    response.into_schema_valid_parsed_answer()
}

pub(crate) fn evaluator_response_json_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "answer": {
                "type": "string",
                "minLength": 1,
                "pattern": "^[^\\r\\n]*$",
            },
            "error": {
                "type": "string",
                "enum": [
                    ERROR_INSUFFICIENT_EVIDENCE,
                    ERROR_INVALID_QUESTION,
                    ERROR_UNPARSABLE,
                ],
            },
            "evidence": {
                "type": "string",
            },
            "qScopeSuggestion": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "string",
                    "minLength": 1,
                    "pattern": "^[^\\r\\n]*$",
                },
            },
        },
        "required": ["evidence", "qScopeSuggestion"],
        "oneOf": [
            {"required": ["answer"], "not": { "required": ["error"] }},
            {"required": ["error"], "not": { "required": ["answer"] }},
        ],
        "additionalProperties": false,
    })
}

pub(crate) fn evaluator_response_output_schema() -> Value {
    // Codex app-server structured output requires every object property to be
    // listed in `required` and represents optional fields as nullable types.
    // Parsing removes those null placeholders; `validate_schema` then applies
    // the Interrogation Policy's canonical exactly-one validation.
    let mut schema = evaluator_response_json_schema();
    let object = schema
        .as_object_mut()
        .expect("evaluator response schema is an object");
    object.insert(
        "required".to_string(),
        json!(["answer", "error", "evidence", "qScopeSuggestion"]),
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
    error["enum"] = json!([
        ERROR_INSUFFICIENT_EVIDENCE,
        ERROR_INVALID_QUESTION,
        ERROR_UNPARSABLE,
        null,
    ]);
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
        rename = "qScopeSuggestion",
        deserialize_with = "deserialize_question_scope_suggestion"
    )]
    pub(crate) question_scope_suggestion: Vec<String>,
}

impl EvaluatorResponseJson {
    fn into_schema_valid_parsed_answer(self) -> Result<ParsedAnswer, String> {
        self.validate_schema()?;
        if let Some(answer) = self.answer {
            // Pass/fail comparison happens after parsing against the
            // expectation's current expected answer.
            return Ok(ParsedAnswer::answer(
                answer,
                self.evidence,
                Some(self.question_scope_suggestion),
            ));
        }
        let error = self
            .error
            .expect("schema validation ensures error is present");
        // The Interrogation Policy schema permits qScopeSuggestion on an error
        // response too. Preserve it for schema fidelity and diagnostics;
        // narrowing policy still consumes suggestions only from answer
        // responses.
        Ok(ParsedAnswer::error_with_question_scope_suggestion(
            error,
            self.evidence,
            Some(self.question_scope_suggestion),
        ))
    }

    pub(crate) fn validate_schema(&self) -> Result<(), String> {
        let has_answer = self.answer.is_some();
        let has_error = self.error.is_some();
        if has_answer == has_error {
            return Err(
                "evaluator response must contain exactly one of answer or error".to_string(),
            );
        }
        if let Some(answer) = self.answer.as_deref() {
            if answer.is_empty() || contains_schema_single_line_violation(answer) {
                return Err("answer must be a non-empty single-line string".to_string());
            }
        }
        if let Some(error) = self.error.as_deref() {
            if !matches!(
                error,
                ERROR_INSUFFICIENT_EVIDENCE | ERROR_INVALID_QUESTION | ERROR_UNPARSABLE
            ) {
                return Err(format!("unsupported evaluator error: {}", error));
            }
        }
        if self.question_scope_suggestion.is_empty() {
            return Err("qScopeSuggestion must contain at least one item".to_string());
        }
        for item in &self.question_scope_suggestion {
            if item.is_empty() || contains_schema_single_line_violation(item) {
                return Err(
                    "qScopeSuggestion items must be non-empty single-line strings".to_string(),
                );
            }
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

fn deserialize_question_scope_suggestion<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: de::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_null() {
        return Err(de::Error::custom("qScopeSuggestion must not be null"));
    }
    Vec::<String>::deserialize(value).map_err(de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::{
        evaluator_response_json_schema, evaluator_response_output_schema,
        parse_evaluator_response_json, EvaluatorResponseJson,
    };
    use serde_json::json;

    #[test]
    fn evaluator_response_json_schema_matches_interrogation_policy() {
        let schema = evaluator_response_json_schema();

        assert_eq!(schema["required"], json!(["evidence", "qScopeSuggestion"]));
        assert_eq!(schema["properties"]["answer"]["type"], json!("string"));
        assert_eq!(schema["properties"]["error"]["type"], json!("string"));
        assert_eq!(schema["oneOf"][0]["required"], json!(["answer"]));
        assert_eq!(schema["oneOf"][1]["required"], json!(["error"]));
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test]
    fn evaluator_response_output_schema_uses_app_server_dialect() {
        let schema = evaluator_response_output_schema();

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
        assert!(schema.get("oneOf").is_none());
    }

    #[test]
    fn evaluator_response_requires_question_scope_suggestion() {
        let error = parse_evaluator_response_json(r#"{"answer":"yes","evidence":"`src/main.rs`"}"#)
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
            .validate_schema()
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
            .validate_schema()
            .unwrap_err()
            .contains("qScopeSuggestion"));
    }

    #[test]
    fn evaluator_response_accepts_required_question_scope_suggestion() {
        let response = parse_evaluator_response_json(
            r#"{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":["src/main.rs"]}"#,
        )
        .unwrap();

        response.validate_schema().unwrap();
        assert_eq!(response.question_scope_suggestion, vec!["src/main.rs"]);
    }

    #[test]
    fn evaluator_response_treats_output_schema_null_placeholders_as_absent() {
        let response = parse_evaluator_response_json(
            r#"{"answer":"yes","error":null,"evidence":"`src/main.rs`","qScopeSuggestion":["src/main.rs"]}"#,
        )
        .unwrap();

        response.validate_schema().unwrap();
        assert_eq!(response.answer.as_deref(), Some("yes"));
        assert_eq!(response.error, None);
    }

    #[test]
    fn evaluator_response_schema_allows_non_crlf_control_chars() {
        let response = parse_evaluator_response_json(
            "{\"answer\":\"yes\\t\",\"evidence\":\"`src/main.rs`\",\"qScopeSuggestion\":[\"src/main.rs\\u0008\"]}",
        )
        .unwrap();

        response.validate_schema().unwrap();
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

        assert!(answer.validate_schema().unwrap_err().contains("answer"));
        assert!(q_scope
            .validate_schema()
            .unwrap_err()
            .contains("qScopeSuggestion"));
    }

    #[test]
    fn evaluator_response_schema_rejects_only_crlf_line_breaks() {
        let schema_valid_unicode_separator = EvaluatorResponseJson {
            answer: Some("yes\u{2028}still schema text".to_string()),
            error: None,
            evidence: "ok".to_string(),
            question_scope_suggestion: vec![".".to_string()],
        };
        assert!(schema_valid_unicode_separator.validate_schema().is_ok());

        let schema_invalid_crlf = EvaluatorResponseJson {
            answer: Some("yes\nno".to_string()),
            error: None,
            evidence: "ok".to_string(),
            question_scope_suggestion: vec![".".to_string()],
        };
        assert!(schema_invalid_crlf.validate_schema().is_err());
    }
}
