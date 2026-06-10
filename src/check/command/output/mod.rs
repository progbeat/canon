// Command output is one component with a small facade. The leaf modules split
// stable stdout/stderr surfaces by output kind while keeping callers away from
// renderer internals.
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
        render_check_agent_messages, start_expectation_report_output, write_cached_non_pass_output,
        write_result_output_without_started_report, SharedCheckOutput,
    };
    use crate::check::core::{CheckRecord, CheckResult};
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

    #[derive(Clone)]
    struct FirstFlushFailsOutput {
        bytes: Arc<Mutex<Vec<u8>>>,
        fail_next_flush: Arc<Mutex<bool>>,
    }

    impl Write for FirstFlushFailsOutput {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            let mut fail_next_flush = self.fail_next_flush.lock().unwrap();
            if *fail_next_flush {
                *fail_next_flush = false;
                return Err(io::Error::other("first flush failed"));
            }
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
    fn started_expectation_report_completes_the_started_result_entry() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = SharedCheckOutput::new(Box::new(CapturedOutput {
            bytes: bytes.clone(),
        }));

        let report = start_expectation_report_output(output, "j");
        let started = captured_string(&bytes);
        assert!(started.starts_with('j'));
        assert!(started.ends_with('.'));
        assert!(!started.contains('\n'));

        report.finish_with_record(&passing_record());
        let completed = captured_string(&bytes);
        assert!(completed.starts_with(&started));
        assert_result_entry(&completed, "OK");
    }

    #[test]
    fn started_expectation_report_renders_full_result_after_unconfirmed_prefix() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = SharedCheckOutput::new(Box::new(FirstFlushFailsOutput {
            bytes: bytes.clone(),
            fail_next_flush: Arc::new(Mutex::new(true)),
        }));

        let report = start_expectation_report_output(output, "j");
        let started = captured_string(&bytes);
        assert_eq!(started, "j.");

        report.finish_with_record(&passing_record());
        let completed = captured_string(&bytes);
        assert!(completed.ends_with("j. OK\n"));
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

    #[test]
    #[should_panic(expected = "all-passed agent message requires no skipped expectations")]
    fn all_passed_agent_message_requires_no_skipped_expectations() {
        let _ = render_check_agent_messages(&[], &[], 0, 0, 1);
    }

    #[test]
    #[should_panic(
        expected = "pass-improvement agent message without remaining issues requires no skipped expectations"
    )]
    fn pass_improvement_commit_message_requires_no_skipped_expectations() {
        let _ = render_check_agent_messages(&[], &[], 1, 0, 1);
    }

    #[test]
    fn repair_message_excludes_all_already_shown_issue_ids() {
        let messages = render_check_agent_messages(&issues(&["a"]), &issues(&["b"]), 0, 0, 0);
        let repair_message = messages
            .iter()
            .find(|message| message.contains("canon show"))
            .expect("repair message should include the follow-up command");

        assert!(repair_message.contains("not:a"));
        assert!(repair_message.contains("not:b"));
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

    fn record_with_result(result: CheckResult, observed: &str) -> CheckRecord {
        CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: 1,
            result,
            question: Some("Does it pass?".to_string()),
            expected_answer: Some("yes".to_string()),
            observed: observed.to_string(),
            error: None,
            evidence: "test evidence".to_string(),
            scope: vec![".".to_string()],
            question_scope_suggestion: None,
            visible_tree_oid: "visible".to_string(),
            id: "11111111111111111111".to_string(),
            display_id: "j".to_string(),
        }
    }
}
