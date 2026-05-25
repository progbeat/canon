use crate::check_output::compact_json_string_array;

const DEVELOPER_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("../../resources/prompts/evaluator_developer_instructions.txt");
pub(crate) const EVALUATOR_BASE_INSTRUCTIONS: &str =
    "You are a read-only canon evaluator. Answer the current turn using only canon's developerInstructions payload, current turn input, and permitted project files. For questions about your developer/dev instructions, consider only that payload.";

pub(crate) fn developer_instructions(scope: &[String]) -> String {
    let scope = compact_json_string_array(scope);
    render_instruction_template(
        DEVELOPER_INSTRUCTIONS_TEMPLATE.trim_end(),
        &[("{{scope}}", &scope)],
    )
}

fn render_instruction_template(template: &str, replacements: &[(&str, &str)]) -> String {
    replacements
        .iter()
        .fold(template.to_owned(), |rendered, (placeholder, value)| {
            rendered.replace(placeholder, value)
        })
}
