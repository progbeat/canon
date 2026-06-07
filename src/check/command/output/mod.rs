mod escape;
mod query;
mod record;
mod shared;
mod summary;
mod usage;

pub(crate) use escape::escape_check_output_text;
pub(crate) use query::write_query_output;
pub(crate) use record::{
    record_requires_human_review, start_check_progress_output,
    write_result_output_without_live_progress, CheckProgressOutput,
};
pub(crate) use shared::{write_stdout_record, SharedCheckOutput};
pub(crate) use summary::{render_check_agent_messages, summary_outcome_counts, write_summary_line};
pub(crate) use usage::render_token_usage_summary;

#[cfg(test)]
mod tests {
    use super::{
        render_check_agent_messages, start_check_progress_output,
        write_result_output_without_live_progress, SharedCheckOutput,
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

    #[test]
    fn check_result_output_without_live_progress_writes_immediate_dot() {
        let mut bytes = Vec::new();
        let mut result_output = Some(&mut bytes as &mut dyn Write);

        write_result_output_without_live_progress(&mut result_output, &passing_record()).unwrap();

        assert_eq!(String::from_utf8(bytes).unwrap(), "j. OK\n");
    }

    #[test]
    fn progress_output_writes_prefix_before_completion() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = SharedCheckOutput::new(Box::new(CapturedOutput {
            bytes: bytes.clone(),
        }));

        let progress = start_check_progress_output(output, "j").unwrap();
        assert_eq!(captured_string(&bytes), "j.");

        progress.finish_with_record(&passing_record()).unwrap();
        assert_eq!(captured_string(&bytes), "j. OK\n");
    }

    #[test]
    fn check_agent_messages_follow_spec_branch_order() {
        assert_eq!(
            render_check_agent_messages(1, 0, 0, 0),
            vec![
                "❕ Verify that the evidence supports the observed answer and answers the expectation question; treat unsupported evidence as a readability issue.",
                "❕ Plan the repair, then run `canon show -- <PATHSPEC>...` for the planned edit paths to identify expectations that may be affected.",
                "❕ Use the matching expectations to avoid regressions while fixing the issues.",
                "▷ Fix the issues and run `canon check` again!"
            ]
        );
        assert_eq!(
            render_check_agent_messages(0, 0, 0, 0),
            vec!["✓ All checks passed. Commit is allowed."]
        );
        assert_eq!(
            render_check_agent_messages(0, 0, 1, 0),
            vec!["▷ +1 pass compared to HEAD. Commit the staged changes NOW!"]
        );
        assert_eq!(
            render_check_agent_messages(1, 0, 2, 0),
            vec![
                "▷ +2 passes compared to HEAD. Commit the staged changes NOW!",
                "❕ Verify that the evidence supports the observed answer and answers the expectation question; treat unsupported evidence as a readability issue.",
                "❕ Plan the repair, then run `canon show -- <PATHSPEC>...` for the planned edit paths to identify expectations that may be affected.",
                "❕ Use the matching expectations to avoid regressions while fixing the issues.",
                "▷ Then fix the remaining issues and run `canon check` again!"
            ]
        );
        assert_eq!(
            render_check_agent_messages(0, 0, 1, 1),
            vec![
                "❕ Verify that the evidence supports the observed answer and answers the expectation question; treat unsupported evidence as a readability issue.",
                "❕ Plan the repair, then run `canon show -- <PATHSPEC>...` for the planned edit paths to identify expectations that may be affected.",
                "❕ Use the matching expectations to avoid regressions while fixing the issues.",
                "▷ Fix the issues and run `canon check` again!"
            ]
        );
    }

    fn captured_string(bytes: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(bytes.lock().unwrap().clone()).unwrap()
    }

    fn passing_record() -> CheckRecord {
        CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: 1,
            result: CheckResult::Pass,
            question: Some("Does it pass?".to_string()),
            expected_answer: Some("yes".to_string()),
            observed: "yes".to_string(),
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
