use crate::check::core::{
    evaluator_response_json_schema, evaluator_response_output_schema_for_requested_short_ids,
    evaluator_response_output_schema_for_schema_scope, parse_evaluator_response_for_short_id,
    parse_evaluator_response_json as parse_response_json_for_short_id,
    parse_evaluator_response_json_for_requested_short_ids, EvaluatorResponseJson,
    EvaluatorResponseParseError, EvaluatorResponseSchemaScope, ParsedAnswer, ANSWER_PATTERN,
    ERROR_INVALID_QUESTION, ERROR_SCOPE_TOO_NARROW,
};
use serde_json::json;

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

#[test] // xpec: mh
fn evaluator_response_requires_the_requested_short_id() {
    let error = parse_evaluator_response_for_short_id(
        &keyed_response(r#"{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":["."]}"#),
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

#[test] // xpec: mh
fn evaluator_response_rejects_already_answered_short_id() {
    let error = parse_evaluator_response_for_short_id(
        &keyed_response(r#"{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":["."]}"#),
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

#[test] // xpec: mh
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

#[test] // xpec: mh
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

#[test] // xpec: mh
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

#[test] // xpec: mh
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
    assert_eq!(
        result_schema["properties"]["qScopeSuggestion"]["items"]["minLength"],
        json!(1)
    );
    assert_eq!(result_schema["oneOf"][0]["required"], json!(["answer"]));
    assert_eq!(result_schema["oneOf"][1]["required"], json!(["error"]));
    assert_eq!(result_schema["additionalProperties"], json!(false));
}

#[test] // xpec: mh
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

#[test] // xpec: mh
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

#[test] // xpec: mh
fn restricted_evaluator_response_output_schema_matches_interrogation_policy() {
    let schema =
        evaluator_response_output_schema_for_schema_scope(EvaluatorResponseSchemaScope::Restricted);
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

#[test] // xpec: mh
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

#[test] // xpec: mh
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

#[test] // xpec: mh
fn full_project_evaluator_response_rejects_scope_too_narrow() {
    let error = parse_evaluator_response(
        r#"{"error":"ScopeTooNarrow","evidence":"scope"}"#,
        EvaluatorResponseSchemaScope::FullProject,
    )
    .unwrap_err();

    assert!(error.to_string().contains("ScopeTooNarrow"));
}

#[test] // xpec: mh
fn restricted_evaluator_response_requires_question_scope_suggestion() {
    let response =
        parse_evaluator_response_json(r#"{"answer":"yes","evidence":"`src/main.rs`"}"#).unwrap();
    let error = response
        .validate_schema(EvaluatorResponseSchemaScope::Restricted)
        .unwrap_err();

    assert!(error.to_string().contains("qScopeSuggestion"));
}

#[test] // xpec: mh
fn full_project_evaluator_response_requires_question_scope_suggestion() {
    let response =
        parse_evaluator_response_json(r#"{"answer":"yes","evidence":"`src/main.rs`"}"#).unwrap();
    let error = response
        .validate_schema(EvaluatorResponseSchemaScope::FullProject)
        .unwrap_err();

    assert!(error.to_string().contains("qScopeSuggestion"));
}

#[test] // xpec: mh
fn evaluator_response_rejects_null_fields() {
    let error = parse_evaluator_response(
        r#"{"answer":"yes","error":null,"evidence":"`src/main.rs`","qScopeSuggestion":null}"#,
        EvaluatorResponseSchemaScope::FullProject,
    )
    .unwrap_err();

    assert!(error.to_string().contains("must not be null"));
}

#[test] // xpec: mh
fn without_question_scope_suggestion_evaluator_response_omits_question_scope_suggestion() {
    let response = parse_evaluator_response(
        r#"{"answer":"yes","evidence":"`src/main.rs`"}"#,
        EvaluatorResponseSchemaScope::WithoutQuestionScopeSuggestion,
    )
    .unwrap();

    assert_eq!(response.observed, "yes");
    assert_eq!(response.question_scope_suggestion, None);
}

#[test] // xpec: mh
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

#[test] // xpec: mh
fn without_question_scope_suggestion_evaluator_response_rejects_question_scope_suggestion() {
    let error = parse_evaluator_response(
        r#"{"answer":"yes","evidence":"`src/main.rs`","qScopeSuggestion":["."]}"#,
        EvaluatorResponseSchemaScope::WithoutQuestionScopeSuggestion,
    )
    .unwrap_err();

    assert!(error.to_string().contains("qScopeSuggestion"));
}

#[test] // xpec: mh
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

#[test] // xpec: mh
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

#[test] // xpec: mh
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

#[test] // xpec: mh
fn evaluator_response_schema_allows_non_crlf_question_scope_chars() {
    let response = parse_evaluator_response_json(
        "{\"answer\":\"yes\",\"evidence\":\"`src/main.rs`\",\"qScopeSuggestion\":[\"src/main.rs\\u0008\"]}",
    )
    .unwrap();

    response
        .validate_schema(EvaluatorResponseSchemaScope::Restricted)
        .unwrap();
}

#[test] // xpec: mh
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

#[test] // xpec: mh
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

#[test] // xpec: mh
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

#[test] // xpec: mh
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
