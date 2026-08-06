use super::escape::{
    push_diff_from_line, push_error_and_evidence_lines, push_escaped_check_output_line,
    push_observed_and_evidence_lines, push_q_scope_suggestion,
};
use super::record::LiveProgressOutput;
use crate::check::core::QueryResult;

pub(crate) fn finish_query_output(
    started_report: LiveProgressOutput,
    result: &QueryResult,
    human_review_reason: Option<&str>,
) -> Result<(), String> {
    // `canon ask` stdout is the started progress report plus this response
    // block. `render_query_output` intentionally renders only the response
    // lines; `finish_with_query_output` first completes the temporary short-ID
    // and progress-timeline line, then appends these response details.
    let output = render_query_output(result, human_review_reason);
    started_report.finish_with_query_output(&output)
}

fn render_query_output(result: &QueryResult, human_review_reason: Option<&str>) -> String {
    let answer = &result.answer;
    let mut output = String::new();
    if let Some(error) = answer.error.as_deref() {
        push_error_and_evidence_lines(&mut output, error, answer.evidence.as_deref());
    } else {
        if let (Some(diff_from), Some(diff_from_tree_oid_abbrev)) = (
            result.diff_from.as_deref(),
            result.diff_from_tree_oid_abbrev.as_deref(),
        ) {
            push_diff_from_line(&mut output, diff_from, diff_from_tree_oid_abbrev);
        }
        push_observed_and_evidence_lines(&mut output, &answer.observed, answer.evidence.as_deref());
        if let Some(suggestion) = answer.q_scope_suggestion.as_deref() {
            push_q_scope_suggestion(&mut output, suggestion);
        }
    }
    if let Some(reason) = human_review_reason {
        push_escaped_check_output_line(&mut output, "review-required", reason);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::finish_query_output;
    use crate::check::command::output::{start_query_report_output, SharedCheckOutput};
    use crate::check::core::{ParsedAnswer, QueryResult};
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    #[test] // xpec: Eg,90
    fn error_query_output_matches_ask_mode_agent_contract() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = SharedCheckOutput::new(Box::new(CapturedOutput {
            bytes: bytes.clone(),
        }));
        let report = start_query_report_output(output, "q");
        let answer = ParsedAnswer::error_with_evidence(
            "first\nsecond".to_string(),
            "line\nbreak".to_string(),
        );

        finish_query_output(report, &query_result(answer), None).unwrap();

        assert_eq!(
            String::from_utf8(bytes.lock().unwrap().clone()).unwrap(),
            "q.\nerror: first\nsecond\nevidence: line\\nbreak\n"
        );
    }

    #[test] // xpec: 2Z,l
    fn review_required_query_output_identifies_the_next_step() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = SharedCheckOutput::new(Box::new(CapturedOutput {
            bytes: bytes.clone(),
        }));
        let report = start_query_report_output(output, "q");
        let answer = ParsedAnswer::error_without_evidence(
            crate::check::core::ERROR_INVALID_QUESTION.to_string(),
        );

        finish_query_output(report, &query_result(answer), Some("invalid question")).unwrap();

        assert_eq!(
            String::from_utf8(bytes.lock().unwrap().clone()).unwrap(),
            "q.\nerror: InvalidQuestion\nreview-required: invalid question\n"
        );
    }

    fn query_result(answer: ParsedAnswer) -> QueryResult {
        QueryResult {
            answer,
            diff_from: None,
            diff_from_tree_oid_abbrev: None,
        }
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
