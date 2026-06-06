use super::escape::escape_check_output_text;
use super::shared::write_stdout_record;
use crate::check::core::types::ParsedAnswer;
use crate::json_util::compact_json_string_array;
use std::io::Write;

pub(crate) fn write_query_output(
    result_output: &mut dyn Write,
    answer: &ParsedAnswer,
) -> Result<(), String> {
    let output = render_query_output(answer);
    write_stdout_record(result_output, output.as_bytes(), "query result")
}

fn render_query_output(answer: &ParsedAnswer) -> String {
    let mut output = String::new();
    if let Some(error) = answer.error.as_deref() {
        output.push_str("Error: ");
        output.push_str(&escape_check_output_text(error));
    } else {
        output.push_str("Observed: ");
        output.push_str(&escape_check_output_text(&answer.answer));
    }
    output.push('\n');
    output.push_str("Evidence: ");
    output.push_str(&escape_check_output_text(&answer.evidence));
    output.push('\n');
    if let Some(suggestion) = answer.q_scope_suggestion.as_deref() {
        output.push_str("Suggested q-scope: ");
        output.push_str(&compact_json_string_array(suggestion));
        output.push('\n');
    }
    output
}
