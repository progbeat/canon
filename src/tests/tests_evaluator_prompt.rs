use super::*;

#[test]
fn evaluator_prompt_is_only_current_question_text() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let prompt = "Permission question?".to_string();
    assert_eq!(prompt, "Permission question?");
    assert!(!prompt.contains("Response format:"));
    assert!(!prompt.contains("answer/error schema"));
    assert!(!prompt.contains("Instructions:"));
    assert!(config.agent.instructions.is_none());
    assert!(!prompt.contains("Current context:"));
    assert!(!prompt.contains("\nQuestion:\n"));
    assert!(!prompt.contains("QUESTION:"));
    assert!(!prompt.contains("\nExpectation:\n"));
    assert!(!prompt.contains("Runtime canon metadata"));
    assert!(!prompt.contains("repository pre-commit hook"));
    assert!(!prompt.contains("core.hooksPath"));
    assert!(!prompt.contains("evaluator default permission profile"));
}

#[test]
fn evaluator_turn_input_is_plain_question_string() {
    let prompt = "Permission question?".to_string();
    let input = evaluator_turn_input(&prompt).unwrap();
    assert_eq!(input, json!("Permission question?"));
    assert_eq!(render_evaluator_turn_input(&input).unwrap(), prompt);
}

#[test]
fn evaluator_turn_uses_strict_json_output_schema() {
    let schema = evaluator_response_output_schema();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"], json!(["evidence"]));
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["answer"]["type"], "string");
    assert_eq!(schema["properties"]["answer"]["minLength"], 1);
    assert_eq!(schema["properties"]["answer"]["pattern"], "^[^\\r\\n]*$");
    assert_eq!(
        schema["properties"]["error"]["enum"],
        json!(["insufficient-evidence", "invalid-question", "unparsable"])
    );
    assert_eq!(schema["properties"]["evidence"]["type"], "string");
    assert!(schema["properties"]["evidence"]["minLength"].is_null());
    assert_eq!(schema["properties"]["qScopeSuggestion"]["type"], "array");
    assert_eq!(schema["properties"]["qScopeSuggestion"]["minItems"], 1);
    assert_eq!(
        schema["properties"]["qScopeSuggestion"]["items"]["type"],
        "string"
    );
    assert_eq!(
        schema["properties"]["qScopeSuggestion"]["items"]["minLength"],
        1
    );
    assert_eq!(
        schema["properties"]["qScopeSuggestion"]["items"]["pattern"],
        "^[^\\r\\n]*$"
    );
    assert_eq!(
        schema["oneOf"],
        json!([
            {"required": ["answer"], "not": { "required": ["error"] }},
            {"required": ["error"], "not": { "required": ["answer"] }}
        ])
    );
}

#[test]
fn evaluator_base_instructions_define_dev_instruction_boundary() {
    assert!(EVALUATOR_BASE_INSTRUCTIONS.contains("developerInstructions payload"));
    assert!(!EVALUATOR_BASE_INSTRUCTIONS.contains("apply_patch"));
}

#[test]
fn developer_instructions_include_builtin_policy_and_ignore_config_instructions() {
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: Answer from files only.
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#,
    )
    .unwrap();
    let instructions = developer_instructions(&full_scope());
    assert_eq!(
        config.agent.instructions.as_deref(),
        Some("Answer from files only.")
    );
    assert!(!instructions.contains("Project-specific evaluator policy loaded from check.yml"));
    assert!(!instructions.contains("Answer from files only."));
    assert!(instructions.contains("Response format:"));
    assert!(instructions.contains("Prefer `rg` and `rg --files`"));
    assert!(instructions.contains("project-relative refs enclosed in backticks"));
    assert!(instructions.contains("For self-contained questions"));
    assert!(instructions.contains("Do not cite proxy evidence"));
    assert!(instructions.contains("qScopeSuggestion"));
    assert!(instructions.contains("narrowest scope"));
    assert!(instructions.contains("answer and justify the whole question"));
    assert!(instructions.contains("missing files in the enforced scope"));
    assert!(instructions.contains("direct evidence from the relevant implementation files"));
    assert!(instructions.contains("Enforced scope: [\".\"]"));
    assert!(instructions.contains("Answer-selection policy:"));
    assert!(!instructions.contains("Instruction-boundary policy"));
}

#[test]
fn developer_instructions_omit_blank_agent_instruction_section() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let instructions = developer_instructions(&full_scope());

    assert!(config.agent.instructions.is_none());
    assert!(!instructions.contains("Project-specific evaluator policy loaded from check.yml"));
}
