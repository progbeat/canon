use crate::check_output::compact_json_string_array;
use crate::config_types::AgentConfig;

const DEVELOPER_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("../resources/prompts/evaluator_developer_instructions.txt");
// Prompt fragments live as tracked text files so policy wording can be reviewed
// without editing Rust string literals; include_str! wires them into the binary.
const EVALUATOR_ANSWER_POLICY: &str =
    include_str!("../resources/prompts/evaluator_answer_policy.txt");
const EVALUATOR_RESPONSE_FORMAT: &str =
    include_str!("../resources/prompts/evaluator_response_format.txt");
pub(crate) const EVALUATOR_BASE_INSTRUCTIONS: &str =
    "You are a read-only canon evaluator. Answer the current turn using only canon's developerInstructions payload, current turn input, and permitted project files. For questions about your developer/dev instructions, consider only that payload.";

pub(crate) fn developer_instructions(agent: &AgentConfig, scope: &[String]) -> String {
    let scope = compact_json_string_array(scope);
    let agent_instructions = custom_agent_instruction_block(agent);
    render_instruction_template(
        DEVELOPER_INSTRUCTIONS_TEMPLATE.trim_end(),
        &[
            ("{{response_format}}", response_format_block()),
            ("{{scope}}", &scope),
            ("{{answer_policy}}", answer_policy()),
            ("{{agent_instructions}}", &agent_instructions),
        ],
    )
}

pub(crate) fn response_format_block() -> &'static str {
    EVALUATOR_RESPONSE_FORMAT
}

fn answer_policy() -> &'static str {
    EVALUATOR_ANSWER_POLICY.trim_end()
}

fn custom_agent_instruction_block(agent: &AgentConfig) -> String {
    let instructions = agent.custom_instructions().trim();
    if instructions.is_empty() {
        String::new()
    } else {
        format!(
            "Project-specific evaluator policy loaded from check.yml:\n{}",
            instructions
        )
    }
}

fn render_instruction_template(template: &str, replacements: &[(&str, &str)]) -> String {
    replacements
        .iter()
        .fold(template.to_owned(), |rendered, (placeholder, value)| {
            rendered.replace(placeholder, value)
        })
}
