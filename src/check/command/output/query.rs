use super::escape::escape_check_output_text;
use super::record::StartedExpectationReportOutput;
use crate::check::core::ParsedAnswer;
use crate::json_util::compact_json_string_array;

pub(crate) fn finish_query_output(
    started_report: StartedExpectationReportOutput,
    answer: &ParsedAnswer,
) -> Result<(), String> {
    // `canon ask` stdout is the started progress report plus this response
    // block. `render_query_output` intentionally renders only the response
    // lines; `finish_with_query_output` first writes the standalone progress
    // timeline line, then appends these `observed`/`error` and `evidence`
    // lines.
    let output = render_query_output(answer);
    started_report.finish_with_query_output(&output)
}

fn render_query_output(answer: &ParsedAnswer) -> String {
    let mut output = String::new();
    if let Some(error) = answer.error.as_deref() {
        output.push_str("error: ");
        output.push_str(&escape_check_output_text(error));
        output.push('\n');
    } else {
        output.push_str("observed: ");
        output.push_str(&escape_check_output_text(&answer.observed));
        output.push('\n');
        output.push_str("evidence: ");
        output.push_str(&escape_check_output_text(&answer.evidence));
        output.push('\n');
        if let Some(suggestion) = answer.question_scope_suggestion.as_deref() {
            output.push_str("q-scope-suggestion: ");
            output.push_str(&compact_json_string_array(suggestion));
            output.push('\n');
        }
    }
    output
}
