use super::{matches_short_id_pattern, EvaluatorResponseSchemaScope, ANSWER_PATTERN};
use serde_json::{json, Value};

#[cfg(test)]
use super::{
    parse_evaluator_response_for_short_id,
    parse_evaluator_response_json as parse_response_json_for_short_id,
    parse_evaluator_response_json_for_requested_short_ids, EvaluatorResponseJson,
    EvaluatorResponseParseError, ParsedAnswer, ERROR_INVALID_QUESTION, ERROR_SCOPE_TOO_NARROW,
};

#[cfg(test)]
pub(crate) fn evaluator_response_json_schema(schema_scope: EvaluatorResponseSchemaScope) -> Value {
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
        "oneOf": [
            {
                "required": ["answer", "evidence"],
                "not": { "required": ["error"] },
            },
            {
                "required": ["error"],
                "not": {
                    "anyOf": [
                        {"required": ["answer"]},
                        {"required": ["evidence"]},
                    ],
                },
            },
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
                // Interrogation Policy's qScopeSuggestion item schema requires
                // non-empty path strings in addition to rejecting CR/LF.
                "minLength": 1,
                "pattern": "^[^\\r\\n]*$",
            },
        });
        if schema_scope.requires_question_scope_suggestion() {
            schema["required"] = json!(["qScopeSuggestion"]);
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

pub(crate) fn evaluator_response_output_schema_for_requested_short_ids(
    schema_scope: EvaluatorResponseSchemaScope,
    short_ids: &[&str],
) -> Value {
    // xpec: w
    assert!(!short_ids.is_empty(), "requested short IDs are required");
    // xpec: w
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
    // The app-server structured-output subset rejects `oneOf`. The common
    // object type states the selected schema restriction directly, while two
    // strict `anyOf` object branches preserve the accepted shape without null
    // transport placeholders: one branch has `answer`, the other has `error`.
    json!({
        "type": "object",
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
    let mut result_properties = vec![result_field];
    if result_field == "answer" {
        result_properties.insert(0, "evidence");
    }
    for key in result_properties {
        branch_properties.insert(
            key.to_string(),
            properties
                .get(key)
                .expect("evaluator response schema property exists")
                .clone(),
        );
    }
    let mut required = if result_field == "answer" {
        vec!["evidence", result_field]
    } else {
        vec![result_field]
    };
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
pub(crate) fn evaluator_response_output_schema_for_schema_scope(
    schema_scope: EvaluatorResponseSchemaScope,
) -> Value {
    evaluator_response_output_schema_for_scope(schema_scope, "q")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_evaluator_response(
        result_json: &str,
        schema_scope: EvaluatorResponseSchemaScope,
    ) -> Result<ParsedAnswer, EvaluatorResponseParseError> {
        parse_evaluator_response_for_short_id(&keyed_response(result_json), schema_scope, "q", &[])
    }

    fn parse_evaluator_response_json(
        result_json: &str,
    ) -> Result<EvaluatorResponseJson, EvaluatorResponseParseError> {
        parse_response_json_for_short_id(&keyed_response(result_json), "q", &[])
    }

    fn keyed_response(result_json: &str) -> String {
        format!(r#"{{"q":{}}}"#, result_json)
    }

    #[test] // xpec: w
    fn evaluator_response_requires_the_requested_short_id() {
        let error = parse_evaluator_response_for_short_id(
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
        assert!(error.to_string().contains("other"));
    }

    #[test] // xpec: w
    fn evaluator_response_rejects_already_answered_short_id() {
        let error = parse_evaluator_response_for_short_id(
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
        assert!(error.to_string().contains("already answered"));
    }

    #[test] // xpec: w
    fn evaluator_response_rejects_unrequested_short_id() {
        let error = parse_evaluator_response_for_short_id(
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
        assert!(error.to_string().contains("unrequested short ID `other`"));
    }

    #[test] // xpec: T,w
    fn evaluator_response_rejects_duplicate_result_field() {
        let error = parse_evaluator_response_json(
            r#"{"answer":"yes","answer":"no","evidence":"duplicate","qScopeSuggestion":["."]}"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("duplicate field"));
    }

    #[test] // xpec: T,w
    fn evaluator_response_rejects_duplicate_short_id() {
        let error = parse_evaluator_response_json_for_requested_short_ids(
            r#"{
            "q":{"answer":"yes","evidence":"first","qScopeSuggestion":["."]},
            "q":{"answer":"no","evidence":"second","qScopeSuggestion":["."]}
        }"#,
            &["q"],
            &[],
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("duplicate evaluator response short ID `q`"));
    }

    #[test] // xpec: w
    fn evaluator_response_output_schema_supports_each_requested_short_id() {
        let schema = evaluator_response_output_schema_for_requested_short_ids(
            EvaluatorResponseSchemaScope::Restricted,
            &["a", "b"],
        );

        assert_eq!(schema["required"], json!(["a", "b"]));
        assert!(schema["properties"].get("a").is_some());
        assert!(schema["properties"].get("b").is_some());
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test] // xpec: w
    fn evaluator_response_parser_accepts_each_requested_short_id() {
        let responses = parse_evaluator_response_json_for_requested_short_ids(
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

    #[test] // xpec: Nt,w
    fn restricted_evaluator_response_json_schema_matches_interrogation_policy() {
        let schema = evaluator_response_json_schema(EvaluatorResponseSchemaScope::Restricted);
        let result_schema = &schema["additionalProperties"];

        assert_eq!(schema["propertyNames"]["pattern"], json!("^[A-Za-z0-9]+$"));
        assert_eq!(result_schema["required"], json!(["qScopeSuggestion"]));
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
        assert_eq!(
            result_schema["properties"]["qScopeSuggestion"]["items"]["minLength"],
            json!(1)
        );
        assert_eq!(
            result_schema["oneOf"][0]["required"],
            json!(["answer", "evidence"])
        );
        assert_eq!(result_schema["oneOf"][1]["required"], json!(["error"]));
        assert_eq!(
            result_schema["oneOf"][1]["not"]["anyOf"],
            json!([
                {"required": ["answer"]},
                {"required": ["evidence"]},
            ])
        );
        assert_eq!(result_schema["additionalProperties"], json!(false));
    }

    #[test] // xpec: Nt,w
    fn full_project_evaluator_response_json_schema_disables_scope_too_narrow() {
        let schema = evaluator_response_json_schema(EvaluatorResponseSchemaScope::FullProject);
        let result_schema = &schema["additionalProperties"];

        assert_eq!(result_schema["required"], json!(["qScopeSuggestion"]));
        assert!(result_schema["properties"]
            .get("qScopeSuggestion")
            .is_some());
        assert_eq!(
            result_schema["properties"]["error"]["enum"],
            json!([ERROR_INVALID_QUESTION])
        );
    }

    #[test] // xpec: Nt,w
    fn without_question_scope_suggestion_evaluator_response_json_schema_omits_question_scope_suggestion(
    ) {
        let schema = evaluator_response_json_schema(
            EvaluatorResponseSchemaScope::WithoutQuestionScopeSuggestion,
        );
        let result_schema = &schema["additionalProperties"];

        assert!(result_schema.get("required").is_none());
        assert!(result_schema["properties"]
            .get("qScopeSuggestion")
            .is_none());
        assert_eq!(
            result_schema["properties"]["error"]["enum"],
            json!([ERROR_INVALID_QUESTION])
        );
    }

    #[test] // xpec: Nt,w
    fn restricted_evaluator_response_output_schema_matches_interrogation_policy() {
        let schema = evaluator_response_output_schema_for_schema_scope(
            EvaluatorResponseSchemaScope::Restricted,
        );
        let result_schema = &schema["properties"]["q"];

        assert_eq!(schema["required"], json!(["q"]));
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(result_schema["type"], json!("object"));
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
            json!(["error", "qScopeSuggestion"])
        );
        assert!(error_branch["properties"].get("answer").is_none());
        assert!(error_branch["properties"].get("evidence").is_none());
    }

    #[test] // xpec: Nt,w
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
            json!(["error", "qScopeSuggestion"])
        );
        assert!(error_branch["properties"].get("evidence").is_none());
    }

    #[test] // xpec: Nt,w
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
        assert_eq!(error_branch["required"], json!(["error"]));
        assert!(error_branch["properties"].get("qScopeSuggestion").is_none());
        assert!(error_branch["properties"].get("evidence").is_none());
    }

    #[test] // xpec: Nt,w
    fn full_project_evaluator_response_rejects_scope_too_narrow() {
        let error = parse_evaluator_response(
            r#"{"error":"ScopeTooNarrow","qScopeSuggestion":["."]}"#,
            EvaluatorResponseSchemaScope::FullProject,
        )
        .unwrap_err();

        assert!(error.to_string().contains("ScopeTooNarrow"));
    }

    #[test] // xpec: Nt
    fn evaluator_error_omits_evidence() {
        let response = parse_evaluator_response(
            r#"{"error":"InvalidQuestion","qScopeSuggestion":["."]}"#,
            EvaluatorResponseSchemaScope::Restricted,
        )
        .unwrap();

        assert_eq!(response.error.as_deref(), Some(ERROR_INVALID_QUESTION));
        assert_eq!(response.evidence, None);
    }

    #[test] // xpec: Nt
    fn evaluator_error_rejects_evidence() {
        let error = parse_evaluator_response(
            r#"{"error":"InvalidQuestion","evidence":"details","qScopeSuggestion":["."]}"#,
            EvaluatorResponseSchemaScope::Restricted,
        )
        .unwrap_err();

        assert!(error.to_string().contains("evidence must be omitted"));
    }

    #[test] // xpec: Nt
    fn evaluator_answer_requires_evidence() {
        let error = parse_evaluator_response(
            r#"{"answer":"yes","qScopeSuggestion":["."]}"#,
            EvaluatorResponseSchemaScope::Restricted,
        )
        .unwrap_err();

        assert!(error.to_string().contains("evidence is required"));
    }

    #[test] // xpec: w
    fn restricted_evaluator_response_requires_question_scope_suggestion() {
        let response =
            parse_evaluator_response_json(r#"{"answer":"yes","evidence":"`src/main.rs`"}"#)
                .unwrap();
        let error = response
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .unwrap_err();

        assert!(error.to_string().contains("qScopeSuggestion"));
    }

    #[test] // xpec: w
    fn full_project_evaluator_response_requires_question_scope_suggestion() {
        let response =
            parse_evaluator_response_json(r#"{"answer":"yes","evidence":"`src/main.rs`"}"#)
                .unwrap();
        let error = response
            .validate_schema(EvaluatorResponseSchemaScope::FullProject)
            .unwrap_err();

        assert!(error.to_string().contains("qScopeSuggestion"));
    }

    #[test] // xpec: w
    fn evaluator_response_rejects_null_fields() {
        let error = parse_evaluator_response(
            r#"{"answer":"yes","error":null,"evidence":"`src/main.rs`","qScopeSuggestion":null}"#,
            EvaluatorResponseSchemaScope::FullProject,
        )
        .unwrap_err();

        assert!(error.to_string().contains("must not be null"));
    }

    #[test] // xpec: w
    fn without_question_scope_suggestion_evaluator_response_omits_question_scope_suggestion() {
        let response = parse_evaluator_response(
            r#"{"answer":"yes","evidence":"`src/main.rs`"}"#,
            EvaluatorResponseSchemaScope::WithoutQuestionScopeSuggestion,
        )
        .unwrap();

        assert_eq!(response.observed, "yes");
        assert_eq!(response.question_scope_suggestion, None);
    }

    #[test] // xpec: w
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

    #[test] // xpec: w
    fn without_question_scope_suggestion_evaluator_response_rejects_question_scope_suggestion() {
        let error = parse_evaluator_response(
            r#"{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":["."]}"#,
            EvaluatorResponseSchemaScope::WithoutQuestionScopeSuggestion,
        )
        .unwrap_err();

        assert!(error.to_string().contains("qScopeSuggestion"));
    }

    #[test] // xpec: w
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

    #[test] // xpec: w
    fn evaluator_response_rejects_empty_question_scope_suggestion_item() {
        let response = parse_evaluator_response_json(
            r#"{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":[""]}"#,
        )
        .unwrap();

        assert!(response
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .unwrap_err()
            .contains("non-empty"));
    }

    #[test] // xpec: w
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

    #[test] // xpec: w
    fn evaluator_response_schema_allows_non_crlf_question_scope_chars() {
        let response = parse_evaluator_response_json(
        "{\"answer\":\"yes\",\"evidence\":\"`src/main.rs`\",\"qScopeSuggestion\":[\"src/main.rs\\u0008\"]}",
    )
    .unwrap();

        response
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .unwrap();
    }

    #[test] // xpec: w
    fn evaluator_response_schema_rejects_answers_outside_answer_pattern() {
        for invalid_answer in ["Rust", "yes\t", "yes\n", "yes\u{2028}still", ""] {
            let response = EvaluatorResponseJson {
                answer: Some(invalid_answer.to_string()),
                error: None,
                evidence: Some("ok".to_string()),
                question_scope_suggestion: Some(vec![".".to_string()]),
            };

            assert!(response
                .validate_schema(EvaluatorResponseSchemaScope::Restricted)
                .unwrap_err()
                .contains("answer"));
        }
    }

    #[test] // xpec: w
    fn evaluator_response_schema_rejects_non_canonical_error_token() {
        let response = EvaluatorResponseJson {
            answer: None,
            error: Some("TechnicalFailure".to_string()),
            evidence: None,
            question_scope_suggestion: Some(vec![".".to_string()]),
        };

        assert!(response
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .unwrap_err()
            .contains("unsupported evaluator error"));
    }

    #[test] // xpec: w
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

    #[test] // xpec: w
    fn evaluator_response_schema_rejects_only_crlf_q_scope_line_breaks() {
        let schema_valid_unicode_separator = EvaluatorResponseJson {
            answer: Some("yes".to_string()),
            error: None,
            evidence: Some("ok".to_string()),
            question_scope_suggestion: Some(vec!["src\u{2028}main.rs".to_string()]),
        };
        assert!(schema_valid_unicode_separator
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .is_ok());

        let schema_invalid_crlf = EvaluatorResponseJson {
            answer: Some("yes".to_string()),
            error: None,
            evidence: Some("ok".to_string()),
            question_scope_suggestion: Some(vec!["src\nmain.rs".to_string()]),
        };
        assert!(schema_invalid_crlf
            .validate_schema(EvaluatorResponseSchemaScope::Restricted)
            .is_err());
    }
}
