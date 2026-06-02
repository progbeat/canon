use crate::json_util::compact_json_string_array;

const DEVELOPER_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("../../resources/prompts/evaluator_developer_instructions.txt");
pub(crate) const EVALUATOR_BASE_INSTRUCTIONS: &str =
    include_str!("../../resources/prompts/evaluator_base_instructions.txt");
const EVALUATOR_TURN_PROMPT_TEMPLATE: &str =
    include_str!("../../resources/prompts/evaluator_turn_prompt.txt");

pub(crate) fn developer_instructions(scope: &[String]) -> String {
    let scope = compact_json_string_array(scope);
    render_resource_template(
        DEVELOPER_INSTRUCTIONS_TEMPLATE.trim_end(),
        &[("{{scope}}", &scope)],
    )
}

pub(crate) fn evaluator_turn_prompt(question: &str) -> String {
    render_resource_template(
        EVALUATOR_TURN_PROMPT_TEMPLATE.trim_end(),
        &[("{{question}}", question)],
    )
}

fn render_resource_template(template: &str, replacements: &[(&str, &str)]) -> String {
    replacements
        .iter()
        .fold(template.to_owned(), |rendered, (placeholder, value)| {
            rendered.replace(placeholder, value)
        })
}
