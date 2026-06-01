use serde_json::Value;

pub(crate) fn evaluator_turn_input(prompt: &str) -> Result<Value, String> {
    Ok(Value::String(prompt.to_string()))
}

pub(crate) fn render_evaluator_turn_input(input: &Value) -> Result<String, String> {
    input
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "evaluator task input must be a string".to_string())
}
