use serde::{de, Deserialize};
use serde_json::Value;

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
        // Interrogation Policy keeps `qScopeSuggestion` schema validation to
        // required non-empty single-line strings. Repository-relative scope
        // syntax is not part of response-schema validity; syntax and semantic
        // sufficiency are later narrowing policy checks, which accept a claim
        // only after an independent answer-producing turn.
        // Interrogation Policy's JSON Schema sets `minItems: 1` for
        // `qScopeSuggestion`, so an empty array is a response-schema error.
        if self.question_scope_suggestion.is_empty() {
            return Err("qScopeSuggestion must contain at least one path".to_string());
        }
        // Each item follows the schema's `minLength: 1` and
        // `pattern: "^[^\\r\\n]*$"` constraints.
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
    use super::EvaluatorResponseJson;

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
