use super::{evaluator_response_output_schema_for_scope, evaluator_response_result_json_schema};
use crate::check::core::evaluator_response::{
    EvaluatorResponseSchemaScope, ERROR_INVALID_QUESTION, ERROR_SCOPE_TOO_NARROW,
};
use serde_json::json;

const ANSWER_DESCRIPTION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/prompts/check/evaluator_response/answer_description.txt"
));
const EVIDENCE_DESCRIPTION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/prompts/check/evaluator_response/evidence_description.txt"
));
const Q_SCOPE_SUGGESTION_DESCRIPTION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/prompts/check/evaluator_response/q_scope_suggestion_description.txt"
));

#[test] // xpec: qv,Ez
fn auto_restricted_evaluator_response_output_schema_matches_interrogation_policy() {
    let schema = evaluator_response_output_schema_for_scope(
        EvaluatorResponseSchemaScope::AutoRestricted,
        "q",
        None,
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
    assert_eq!(
        answer_branch["properties"]["evidence"],
        json!({
            "type": "string",
            "description": EVIDENCE_DESCRIPTION.trim(),
        })
    );
    assert_eq!(
        answer_branch["properties"]["answer"]["description"],
        json!(ANSWER_DESCRIPTION.trim())
    );
    assert_eq!(
        answer_branch["properties"]["qScopeSuggestion"]["description"],
        json!(Q_SCOPE_SUGGESTION_DESCRIPTION.trim())
    );
    assert!(answer_branch["properties"].get("error").is_none());
    assert_eq!(
        error_branch["required"],
        json!(["error", "qScopeSuggestion"])
    );
    assert!(error_branch["properties"].get("answer").is_none());
    assert!(error_branch["properties"].get("evidence").is_none());
}

#[test] // xpec: qv
fn selected_auto_schema_requires_q_scope_suggestion_for_both_result_branches() {
    let schema =
        evaluator_response_result_json_schema(EvaluatorResponseSchemaScope::AutoRestricted);

    assert_eq!(schema["required"], json!(["qScopeSuggestion"]));
    assert_eq!(schema["oneOf"].as_array().map(Vec::len), Some(2));
}

#[test] // xpec: qv,Eg
fn auto_full_project_schema_requires_suggestion_on_answer_and_error() {
    let schema = evaluator_response_output_schema_for_scope(
        EvaluatorResponseSchemaScope::AutoFullProject,
        "q",
        None,
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

#[test] // xpec: qv
fn fixed_q_scope_and_no_hidden_files_output_schemas_omit_negotiation_fields() {
    for schema_scope in [
        EvaluatorResponseSchemaScope::FixedQScope,
        EvaluatorResponseSchemaScope::NoHiddenFiles,
    ] {
        let schema = evaluator_response_output_schema_for_scope(schema_scope, "q", None);
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
}

#[test] // xpec: X
fn diff_target_output_schema_presents_default_answer_as_prior_evaluation() {
    let schema = evaluator_response_output_schema_for_scope(
        EvaluatorResponseSchemaScope::AutoFullProject,
        "X",
        Some("no"),
    );
    let result_schema = &schema["properties"]["X"];
    let description = result_schema["description"]
        .as_str()
        .expect("diff-target result schema has a description");

    assert!(description.contains("Prior evaluation"));
    assert_eq!(description.lines().last(), Some("\"no\""));
}
