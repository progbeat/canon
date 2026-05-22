use super::*;

#[test]
fn evaluator_prompt_is_only_current_question_text() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let prompt = "Permission question?".to_string();
    let response_format_heading = response_format_block().lines().next().unwrap();
    assert_eq!(prompt, "Permission question?");
    assert!(!prompt.contains(response_format_heading));
    assert!(!prompt.contains("ANSWER: <single-line answer>"));
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
    assert_eq!(schema["required"], json!(["answer", "evidence", "scope"]));
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["answer"]["type"], "string");
    assert_eq!(schema["properties"]["answer"]["pattern"], "^[^\\r\\n]*$");
    assert_eq!(schema["properties"]["evidence"]["type"], "string");
    assert_eq!(schema["properties"]["scope"]["type"], "array");
    assert_eq!(schema["properties"]["scope"]["minItems"], 1);
    assert_eq!(schema["properties"]["scope"]["items"]["type"], "string");
    assert_eq!(schema["properties"]["scope"]["items"]["minLength"], 1);
    assert_eq!(
        schema["properties"]["scope"]["items"]["pattern"],
        "^[^\\r\\n]*$"
    );
}

#[test]
fn evaluator_base_instructions_define_dev_instruction_boundary() {
    assert!(EVALUATOR_BASE_INSTRUCTIONS.contains("developerInstructions payload"));
    assert!(!EVALUATOR_BASE_INSTRUCTIONS.contains("apply_patch"));
}

#[test]
fn developer_instructions_include_builtin_policy_and_optional_agent_instructions() {
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
    let instructions = developer_instructions(&config.agent, &full_scope());
    let answer_policy =
        include_str!("../../resources/prompts/evaluator_answer_policy.txt").trim_end();
    assert!(instructions.contains("Project-specific evaluator policy loaded from check.yml"));
    assert!(instructions.contains("Answer from files only."));
    assert!(instructions.contains("Prefer `rg` and `rg --files`"));
    assert!(instructions.contains("Correct answer: +5"));
    assert!(instructions.contains(response_format_block()));
    assert!(instructions.contains("project-relative refs enclosed in backticks"));
    assert!(instructions.contains("Never include a path outside the enforced scope"));
    assert!(instructions.contains("Embedded question text loaded from a denied file"));
    assert!(instructions.contains(answer_policy));
    assert!(!instructions.contains("Instruction-boundary policy"));
}

#[test]
fn developer_instructions_omit_blank_agent_instruction_section() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let instructions = developer_instructions(&config.agent, &full_scope());

    assert!(config.agent.instructions.is_none());
    assert!(!instructions.contains("Project-specific evaluator policy loaded from check.yml"));
}
