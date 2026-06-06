mod escape;
mod progress;
mod query;
mod record;
mod shared;
mod summary;
mod usage;

pub(crate) use progress::{start_check_progress_output, CheckProgressOutput};
pub(crate) use query::write_query_output;
pub(crate) use record::{record_requires_human_review, write_and_flush_result_output};
pub(crate) use shared::{write_stdout_line_record, SharedCheckOutput};
pub(crate) use summary::{render_check_agent_messages, summary_outcome_counts, write_summary_line};
pub(crate) use usage::render_token_usage_summary;

#[cfg(test)]
mod tests {
    use super::{
        render_check_agent_messages, start_check_progress_output, write_and_flush_result_output,
        SharedCheckOutput,
    };
    use crate::check::core::types::{CheckRecord, CheckResult};
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

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
    fn check_result_output_rounds_progress_dots_to_elapsed_minutes() {
        let mut bytes = Vec::new();
        let mut result_output = Some(&mut bytes as &mut dyn Write);

        write_and_flush_result_output(
            &mut result_output,
            &passing_record(),
            Duration::from_secs(60) + Duration::from_nanos(1),
        )
        .unwrap();

        assert_eq!(String::from_utf8(bytes).unwrap(), "j.. OK\n");
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
            vec!["▷ Fix the issues and run `canon check` again!"]
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
                "▷ Then fix the remaining issues and run `canon check` again!"
            ]
        );
        assert_eq!(
            render_check_agent_messages(0, 0, 1, 1),
            vec!["▷ Fix the issues and run `canon check` again!"]
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
            prompt: Some("Does it pass?".to_string()),
            expected: Some("yes".to_string()),
            observed: "yes".to_string(),
            error: None,
            evidence: "test evidence".to_string(),
            scope: vec![".".to_string()],
            suggested_q_scope: None,
            visible_tree_oid: "visible".to_string(),
            id: "11111111111111111111".to_string(),
            display_id: "j".to_string(),
            cache_key: None,
        }
    }
}
