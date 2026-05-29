use serde_json::{json, Value};

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn evaluator_response_output_schema() -> Value {
    // This schema mirrors the canon interrogation response contract. The parser
    // enforces it at runtime, rejects surrounding prose, and normalizes invalid
    // JSON/schema mismatches into an `unparsable` evaluator error.
    // The answer vocabulary is intentionally not enumerated here: any
    // schema-valid single-line answer is an observed answer, and expectation
    // comparison decides pass versus fail.
    json!({
        "type": "object",
        "properties": {
            "answer": {
                "type": "string",
                "minLength": 1,
                "pattern": "^[^\\r\\n]*$"
            },
            "error": {
                "type": "string",
                "enum": ["insufficient-evidence", "invalid-question", "unparsable"]
            },
            // Interrogation Policy intentionally keeps `evidence` as a free
            // string in the JSON schema. The evaluator instructions carry the
            // semantic requirement that evidence directly justify the answer
            // and cite non-proxy project evidence when project evidence is
            // used.
            "evidence": {
                "type": "string"
            },
            "qScopeSuggestion": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "string",
                    "minLength": 1,
                    "pattern": "^[^\\r\\n]*$"
                }
            }
        },
        "required": ["evidence"],
        "oneOf": [
            {"required": ["answer"], "not": { "required": ["error"] }},
            {"required": ["error"], "not": { "required": ["answer"] }}
        ],
        "additionalProperties": false
    })
}

pub(crate) fn app_server_evaluator_response_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "answer": { "type": ["string", "null"] },
            "error": { "type": ["string", "null"] },
            "evidence": { "type": "string" },
            "qScopeSuggestion": {
                "type": ["array", "null"],
                "minItems": 1,
                "items": {
                    "type": "string",
                    "minLength": 1,
                    "pattern": "^[^\\r\\n]*$"
                }
            }
        },
        "required": ["answer", "error", "evidence", "qScopeSuggestion"],
        "additionalProperties": false
    })
}

pub(crate) fn evaluator_turn_input(prompt: &str) -> Result<Value, String> {
    Ok(Value::String(prompt.to_string()))
}

pub(crate) fn render_evaluator_turn_input(input: &Value) -> Result<String, String> {
    input
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "evaluator task input must be a string".to_string())
}
