// Command output is one component with a small facade. The leaf modules split
// stable stdout/stderr surfaces by output kind while keeping callers away from
// renderer internals.
// `canon check` uses this facade instead of `crate::output` because live
// progress shares stdout across threads; every exported writer flushes through
// `shared::write_stdout_record` or `SharedCheckOutput` as soon as a record,
// summary, query answer, agent message, or progress dot is eligible.
mod escape;
mod query;
mod record;
mod shared;
mod summary;
mod usage;

pub(crate) use escape::escape_check_output_text;
pub(crate) use query::write_query_output;
pub(crate) use record::{
    start_expectation_report_output, write_cached_non_pass_output,
    write_result_output_without_started_report, StartedExpectationReportOutput,
};
pub(crate) use shared::{write_stdout_record, SharedCheckOutput};
pub(crate) use summary::{render_check_agent_messages, summary_outcome_counts, write_summary_line};
pub(crate) use usage::render_token_usage_summary;

#[cfg(test)]
mod tests {
    use super::{
        render_check_agent_messages, start_expectation_report_output, summary_outcome_counts,
        write_cached_non_pass_output, write_result_output_without_started_report,
        SharedCheckOutput,
    };
    use crate::check::core::{
        CachedExpectation, CheckRecord, CheckResult, CheckRunReport, ERROR_SCOPE_TOO_NARROW,
    };
    use crate::check::SelectedExpectation;
    use crate::config_types::AgentConfig;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
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

    #[test]
    fn non_live_report_result_output_matches_documented_record_shape() {
        let mut bytes = Vec::new();
        let mut result_output = Some(&mut bytes as &mut dyn Write);

        write_result_output_without_started_report(&mut result_output, &passing_record()).unwrap();

        let rendered = String::from_utf8(bytes).unwrap();
        assert_result_entry(&rendered, "OK");
    }

    #[test]
    fn cached_non_pass_output_matches_documented_record_shape() {
        let mut bytes = Vec::new();
        let mut result_output = Some(&mut bytes as &mut dyn Write);

        write_cached_non_pass_output(&mut result_output, &failed_record()).unwrap();

        let rendered = String::from_utf8(bytes).unwrap();
        assert_result_entry(&rendered, "FAILED");
    }

    #[test]
    fn summary_counts_cached_failures_separately_from_current_run_errors() {
        let evaluated_error = review_record_with_id("11111111111111111111", "j");
        let cached_failure = cached_expectation(failed_record_with_id("22222222222222222222", "k"));
        let cached_error = cached_expectation(review_record_with_id("33333333333333333333", "l"));
        let report = CheckRunReport {
            records: vec![evaluated_error],
            cached: vec![cached_failure, cached_error],
            skipped: 0,
        };

        let counts = summary_outcome_counts(&report);

        assert_eq!(counts.passed, 0);
        assert_eq!(counts.failed, 1);
        assert_eq!(counts.errors, 1);
    }

    #[test]
    fn live_report_result_output_matches_documented_record_shape() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = SharedCheckOutput::new(Box::new(CapturedOutput {
            bytes: bytes.clone(),
        }));

        let report = start_expectation_report_output(output, "j");
        report.finish_with_record(&passing_record());

        let completed = captured_string(&bytes);
        assert_result_entry(&completed, "OK");
    }

    #[test]
    fn agent_messages_cover_documented_actions() {
        assert!(has_action(
            &render_check_agent_messages(&issues(&["a"]), &[], 0, 0, 0),
            "Fix the issues"
        ));
        assert!(has_action(
            &render_check_agent_messages(&[], &[], 0, 0, 0),
            "All checks passed"
        ));
        assert!(has_action(
            &render_check_agent_messages(&[], &[], 1, 0, 0),
            "Commit the staged changes"
        ));
        assert!(has_action(
            &render_check_agent_messages(&issues(&["a"]), &[], 2, 0, 1),
            "Then fix the remaining issues"
        ));
        assert!(!has_action(
            &render_check_agent_messages(&issues(&["a"]), &[], 1, 1, 0),
            "Commit the staged changes"
        ));
    }

    fn issues(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    fn has_action(messages: &[String], action: &str) -> bool {
        messages.iter().any(|message| message.contains(action))
    }

    fn assert_result_entry(rendered: &str, status: &str) {
        let first_line = rendered.lines().next().expect("result entry line");
        let (id_and_dots, observed_status) = first_line
            .split_once(' ')
            .expect("result entry separates id/dots from status");
        assert_eq!(id_and_dots.trim_end_matches('.'), "j");
        assert!(id_and_dots.ends_with('.'));
        assert_eq!(observed_status, status);
    }

    fn captured_string(bytes: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(bytes.lock().unwrap().clone()).unwrap()
    }

    fn passing_record() -> CheckRecord {
        record_with_result(CheckResult::Pass, "yes")
    }

    fn failed_record() -> CheckRecord {
        record_with_result(CheckResult::Fail, "no")
    }

    fn failed_record_with_id(id: &str, display_id: &str) -> CheckRecord {
        record_with_identity(CheckResult::Fail, "no", None, id, display_id)
    }

    fn review_record_with_id(id: &str, display_id: &str) -> CheckRecord {
        record_with_identity(
            CheckResult::Fail,
            "",
            Some(ERROR_SCOPE_TOO_NARROW),
            id,
            display_id,
        )
    }

    fn cached_expectation(record: CheckRecord) -> CachedExpectation {
        CachedExpectation {
            expectation: SelectedExpectation {
                number: record.number,
                id: record.id.clone(),
                display_id: record.display_id.clone(),
                question: record.question_text().to_string(),
                expected_answer: record.expected_answer_text().unwrap_or("yes").to_string(),
                instructions: String::new(),
                diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
                target: None,
                question_answer_only: true,
                agent: AgentConfig::default(),
                cooldown: None,
            },
            record,
        }
    }

    fn record_with_result(result: CheckResult, observed: &str) -> CheckRecord {
        record_with_identity(result, observed, None, "11111111111111111111", "j")
    }

    fn record_with_identity(
        result: CheckResult,
        observed: &str,
        error: Option<&str>,
        id: &str,
        display_id: &str,
    ) -> CheckRecord {
        CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: 1,
            result,
            question: Some("Does it pass?".to_string()),
            expected_answer: Some("yes".to_string()),
            observed: observed.to_string(),
            error: error.map(str::to_string),
            evidence: "test evidence".to_string(),
            scope: vec![".".to_string()],
            question_scope_suggestion: None,
            visible_tree_oid: "visible".to_string(),
            id: id.to_string(),
            display_id: display_id.to_string(),
        }
    }
}
