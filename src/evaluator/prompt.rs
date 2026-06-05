use crate::json_util::compact_json_string_array;

// These resource files are the Canon-owned interrogation prompt/instruction
// templates. User-authored expectation questions are runtime data inserted into
// the turn prompt template.
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

#[cfg(test)]
mod tests {
    use super::{developer_instructions, EVALUATOR_BASE_INSTRUCTIONS};

    #[test]
    fn base_instructions_prohibit_status_text() {
        assert!(EVALUATOR_BASE_INSTRUCTIONS.contains("Do not announce skills"));
        assert!(EVALUATOR_BASE_INSTRUCTIONS.contains("only the JSON object"));
        assert!(EVALUATOR_BASE_INSTRUCTIONS.contains("request a shell command or tool call"));
        assert!(EVALUATOR_BASE_INSTRUCTIONS.contains(r#"{"tool":...,"parameters":...}"#));
        assert!(EVALUATOR_BASE_INSTRUCTIONS.contains("I'll inspect"));
    }

    #[test]
    fn developer_instructions_define_topic_neutral_evidence_threshold() {
        let instructions = developer_instructions(&[".".to_string()]);

        assert!(instructions.contains("visible files and question text do not prove"));
        assert!(instructions.contains("Relevant direct reads/searches"));
        assert!(instructions.contains("do not require a literal exhaustive audit"));
        assert!(!instructions.contains("answer `no` to"));
        assert!(instructions.contains("text before or after the JSON is invalid"));
        assert!(instructions.contains("tool-request JSON"));
        assert!(instructions.contains(r#"{"tool":...}"#));
        assert!(instructions.contains(r#"{"command":...}"#));
        assert!(instructions.contains("first non-whitespace character must be `{`"));
        assert!(instructions.contains("leading inspection summaries"));
        assert!(instructions.contains("backslash immediately before a backtick"));
    }
}
