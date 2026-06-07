pub(super) fn render_generator_expectation_question(
    generated_question_format: &str,
    content: &str,
) -> String {
    // The expectations spec defines q_template rendering as plain
    // `{{content}}` substitution to produce user-authored expectation
    // questions. This is deliberately separate from Canon-owned evaluator
    // prompt/instruction templates, which are loaded only by
    // `evaluator::protocol` from `resources/prompts/`.
    generated_question_format.replace("{{content}}", content)
}
