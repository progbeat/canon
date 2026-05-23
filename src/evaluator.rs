use crate::output::write_stdout_line;
use serde_json::{json, Value};

pub(crate) fn evaluator_response_output_schema() -> Value {
    // This schema is the app-server first pass for the interrogation response
    // contract. The parser still enforces constraints JSON Schema cannot express
    // safely here: exact top-level key order, no surrounding prose, and
    // canonical scope normalization/parent-path reduction. Semantic scope
    // sufficiency is not a JSON-shape property: the developer instructions tell
    // the evaluator to return the smallest sufficient scope, and the check
    // interrogation policy independently verifies any strict narrowing before
    // writing that narrower scope to answer history.
    // The answer vocabulary is intentionally not enumerated here: most canon
    // expectations use yes/no/options, but free-form exact single-line answers
    // such as "Rust" are valid when the expectation asks for one.
    json!({
        "type": "object",
        "properties": {
            "answer": {
                "type": "string",
                "pattern": "^[^\\r\\n]*$"
            },
            "evidence": { "type": "string" },
            "scope": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "string",
                    "minLength": 1,
                    "pattern": "^[^\\r\\n]*$"
                }
            }
        },
        "required": ["answer", "evidence", "scope"],
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

pub(crate) fn print_help() -> Result<(), String> {
    for line in [
        "canon - AI linter for project expectations",
        "",
        "Usage:",
        "  canon init",
        "  canon hook install",
        "  canon hook uninstall",
        "  canon check [-c|--config <path>] [--all] [--ignore-cache] [--ignore-cooldown] [expectation selectors...]",
        "  canon check -q <question> [-s|--scope <path>...]",
        "  canon gate",
    ] {
        write_stdout_line(line)?;
    }
    Ok(())
}
