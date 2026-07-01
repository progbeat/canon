use super::escape::escape_check_output_text;
use super::record::StartedExpectationReportOutput;
use crate::check::core::ParsedAnswer;
use crate::json_util::compact_json_string_array;

pub(crate) fn finish_query_output(
    started_report: StartedExpectationReportOutput,
    answer: &ParsedAnswer,
) -> Result<(), String> {
    // `canon ask` asks one question and writes this query answer. It does
    // not derive an expectation result, so it reports the evaluator response
    // after the standalone progress timeline.
    let output = render_query_output(answer);
    started_report.finish_with_query_output(&output)
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
    if let Some(suggestion) = answer.question_scope_suggestion.as_deref() {
        output.push_str("Suggested q-scope: ");
        output.push_str(&compact_json_string_array(suggestion));
        output.push('\n');
    }
    output
}
