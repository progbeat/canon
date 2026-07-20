use super::escape::push_escaped_check_output_line;
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
        output.push_str(error);
        output.push('\n');
    } else {
        push_escaped_check_output_line(&mut output, "observed", &answer.observed);
    }
    if let Some(evidence) = answer.evidence.as_deref() {
        push_escaped_check_output_line(&mut output, "evidence", evidence);
    }
    if answer.error.is_none() {
        if let Some(suggestion) = answer.question_scope_suggestion.as_deref() {
            output.push_str("q-scope-suggestion: ");
            output.push_str(&compact_json_string_array(suggestion));
            output.push('\n');
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{finish_query_output, render_query_output};
    use crate::check::command::output::{start_query_report_output, SharedCheckOutput};
    use crate::check::core::ParsedAnswer;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    #[test] // xpec: RU
    fn query_error_output_keeps_evidence() {
        let answer = ParsedAnswer::error("unparsable".to_string(), "invalid JSON".to_string());

        assert_eq!(
            render_query_output(&answer),
            "error: unparsable\nevidence: invalid JSON\n"
        );
    }

    #[test] // xpec: 90
    fn query_error_output_prints_error_without_inline_escaping() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = SharedCheckOutput::new(Box::new(CapturedOutput {
            bytes: bytes.clone(),
        }));
        let report = start_query_report_output(output);
        let answer = ParsedAnswer::error("first\nsecond".to_string(), "line\nbreak".to_string());

        finish_query_output(report, &answer).unwrap();

        assert_eq!(
            String::from_utf8(bytes.lock().unwrap().clone()).unwrap(),
            ".\nerror: first\nsecond\nevidence: line\\nbreak\n"
        );
    }

    struct CapturedOutput {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for CapturedOutput {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
