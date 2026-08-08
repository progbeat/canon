mod output;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) use output::evaluator_response_output_schema_for_exact_requested_short_ids;
pub(crate) use output::evaluator_response_output_schema_for_scope;

use super::{
    EvaluatorResponseSchemaScope, ANSWER_PATTERN, Q_SCOPE_SUGGESTION_ITEM_MIN_LENGTH,
    Q_SCOPE_SUGGESTION_ITEM_PATTERN, Q_SCOPE_SUGGESTION_MIN_ITEMS,
};
use serde_json::{json, Value};

// Schema descriptions carry semantic response requirements that JSON Schema
// keywords cannot express. Runtime parsing mirrors only the machine-checkable
// keywords; it must not infer presence claims from answer polarity or prose.
const ANSWER_DESCRIPTION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/prompts/check/evaluator_response/answer_description.txt"
));
pub(super) const EVIDENCE_DESCRIPTION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/prompts/check/evaluator_response/evidence_description.txt"
));
pub(super) const DIFF_EVIDENCE_DESCRIPTION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/prompts/check/evaluator_response/diff_evidence_description.txt"
));
pub(super) const DIFF_PRIOR_EVALUATION_DESCRIPTION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/prompts/check/evaluator_response/diff_prior_evaluation_description.txt"
));
const Q_SCOPE_SUGGESTION_DESCRIPTION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/prompts/check/evaluator_response/q_scope_suggestion_description.txt"
));

pub(super) fn evaluator_response_result_json_schema(
    schema_scope: EvaluatorResponseSchemaScope,
) -> Value {
    // [MH,qv] An agent turn emits this evaluation response itself, so the
    // selected schema's string member is already the resolved answer domain,
    // not a pre-response scalar source. JSON such as {"answer": 7} is outside
    // the selected response schema and never becomes an evaluation response.
    // Actual non-string answer sources are normalized before this boundary;
    // for example, the shell producer turns its integer exit code into a
    // String before constructing `EvaluationAnswer`.
    let mut schema = json!({
        "type": "object",
        "properties": {
            "answer": {
                "type": "string",
                "pattern": ANSWER_PATTERN,
                "description": ANSWER_DESCRIPTION.trim(),
            },
            "error": {
                "type": "string",
                "enum": schema_scope.error_enum(),
            },
            "evidence": {
                "type": "string",
                "description": EVIDENCE_DESCRIPTION.trim(),
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
    if schema_scope
        .q_scope_suggestion_policy()
        .requires_agent_q_scope_suggestion()
    {
        // [Eg,qv] The pseudocode response field is optional because it models
        // the union of all response modes. This selected auto schema is exact:
        // its object-level `required` applies to both oneOf branches, so answer
        // and error responses both carry qScopeSuggestion. The evaluator
        // policy's conditional use of a suggestion after an answer does not
        // weaken that presence rule. Fixed/no-hidden schemas instead omit the
        // property entirely.
        schema["properties"]["qScopeSuggestion"] = json!({
            "type": "array",
            "minItems": Q_SCOPE_SUGGESTION_MIN_ITEMS,
            "description": Q_SCOPE_SUGGESTION_DESCRIPTION.trim(),
            "items": {
                "type": "string",
                // Interrogation Policy's qScopeSuggestion item schema requires
                // non-empty path strings in addition to rejecting CR/LF.
                "minLength": Q_SCOPE_SUGGESTION_ITEM_MIN_LENGTH,
                "pattern": Q_SCOPE_SUGGESTION_ITEM_PATTERN,
            },
        });
        schema["required"] = json!(["qScopeSuggestion"]);
    }
    schema
}
