use super::{
    evaluator_response_result_json_schema, DIFF_EVIDENCE_DESCRIPTION,
    DIFF_PRIOR_EVALUATION_DESCRIPTION, EVIDENCE_DESCRIPTION,
};
use crate::check::core::evaluator_response::{
    parse::matches_short_id_pattern, EvaluatorResponseSchemaScope,
};
use serde_json::{json, Value};

pub(crate) fn evaluator_response_output_schema_for_scope(
    schema_scope: EvaluatorResponseSchemaScope,
    short_id: &str,
    diff_target_prior_answer: Option<&str>,
) -> Value {
    evaluator_response_output_schema_for_exact_requested_short_ids_and_target(
        schema_scope,
        &[short_id],
        diff_target_prior_answer,
    )
}

#[cfg(test)]
pub(in crate::check::core::evaluator_response) fn evaluator_response_output_schema_for_exact_requested_short_ids(
    schema_scope: EvaluatorResponseSchemaScope,
    exact_requested_short_ids: &[&str],
) -> Value {
    evaluator_response_output_schema_for_exact_requested_short_ids_and_target(
        schema_scope,
        exact_requested_short_ids,
        None,
    )
}

fn evaluator_response_output_schema_for_exact_requested_short_ids_and_target(
    schema_scope: EvaluatorResponseSchemaScope,
    exact_requested_short_ids: &[&str],
    diff_target_prior_answer: Option<&str>,
) -> Value {
    // [qv] A rendered turn names the complete set of unanswered interrogations
    // requested from that turn. At the transport boundary, instantiate the
    // generic short-ID property shape with exactly those keys so an invalid
    // extra key cannot become an object returned by a structured-output turn.
    // The parser independently classifies extra keys in any response that does
    // reach it (for example, through a transport without equivalent schema
    // enforcement) as short-ID mismatches. Both boundaries therefore enforce
    // the same selected-schema restriction without accepting an extra result.
    // xpec: qv
    assert!(
        !exact_requested_short_ids.is_empty(),
        "requested short IDs are required"
    );
    // xpec: qv
    assert!(
        exact_requested_short_ids
            .iter()
            .all(|short_id| matches_short_id_pattern(short_id)),
        "requested short IDs must match the evaluator response pattern"
    );
    let result_schema =
        evaluator_response_output_result_json_schema(schema_scope, diff_target_prior_answer);
    let required = exact_requested_short_ids.to_vec();
    let mut properties = serde_json::Map::new();
    for short_id in exact_requested_short_ids {
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
    diff_target_prior_answer: Option<&str>,
) -> Value {
    let mut schema = evaluator_response_result_json_schema(schema_scope);
    if let Some(prior_answer) = diff_target_prior_answer {
        schema["properties"]["evidence"]["description"] = json!(format!(
            "{}\n{}",
            EVIDENCE_DESCRIPTION.trim(),
            DIFF_EVIDENCE_DESCRIPTION.trim()
        ));
        schema["description"] = json!(format!(
            "{}\n{}",
            DIFF_PRIOR_EVALUATION_DESCRIPTION.trim(),
            serde_json::to_string(prior_answer).expect("string serialization cannot fail")
        ));
    }
    let properties = schema["properties"]
        .as_object()
        .expect("evaluator response schema has properties");
    // The app-server structured-output subset rejects `oneOf`. The common
    // object type states the selected schema restriction directly, while two
    // strict `anyOf` object branches preserve the accepted shape without null
    // transport placeholders: one branch has `answer`, the other has `error`.
    let mut output_schema = json!({
        "type": "object",
        "anyOf": [
            evaluator_response_output_branch_schema(properties, schema_scope, "answer"),
            evaluator_response_output_branch_schema(properties, schema_scope, "error"),
        ],
    });
    if let Some(description) = schema.get("description") {
        output_schema["description"] = description.clone();
    }
    output_schema
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
    if schema_scope
        .q_scope_suggestion_policy()
        .requires_agent_q_scope_suggestion()
    {
        // [Eg,qv] Add this required property to both transport-supported
        // branches: `result_field` is either answer or error.
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
