mod escape;
mod query;
mod record;
mod shared;
mod summary;
mod usage;

pub(crate) use escape::escape_check_output_text;
pub(crate) use query::write_query_output;
pub(crate) use record::{
    start_live_check_progress_output, write_result_output_without_live_progress,
    LiveCheckProgressOutput,
};
pub(crate) use shared::{write_stdout_record, SharedCheckOutput};
pub(crate) use summary::{render_check_agent_messages, summary_outcome_counts, write_summary_line};
pub(crate) use usage::render_token_usage_summary;

#[cfg(test)]
mod tests {
    use super::{
        render_check_agent_messages, start_live_check_progress_output,
        write_result_output_without_live_progress, SharedCheckOutput,
    };
    use crate::check::core::{CheckRecord, CheckResult};
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    #[derive(Debug, PartialEq, Eq)]
    enum AgentMessageKind {
        AllClear,
        CommitNotice,
        RepairInstruction,
        FixIssues,
        ThenFixRemaining,
    }

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
    fn check_result_output_without_live_progress_writes_no_dot() {
        let mut bytes = Vec::new();
        let mut result_output = Some(&mut bytes as &mut dyn Write);

        write_result_output_without_live_progress(&mut result_output, &passing_record()).unwrap();

        let rendered = String::from_utf8(bytes).unwrap();
        assert!(rendered.starts_with("j "));
        assert!(rendered.ends_with("OK\n"));
        assert!(!rendered.starts_with("j."));
    }

    #[test]
    fn live_progress_output_writes_prefix_before_completion() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = SharedCheckOutput::new(Box::new(CapturedOutput {
            bytes: bytes.clone(),
        }));

        let progress = start_live_check_progress_output(output, "j").unwrap();
        let started = captured_string(&bytes);
        assert!(started.starts_with('j'));
        assert!(started.ends_with('.'));
        assert!(!started.contains('\n'));

        progress.finish_with_record(&passing_record()).unwrap();
        let completed = captured_string(&bytes);
        assert!(completed.starts_with(&started));
        assert!(completed.ends_with("OK\n"));
    }

    #[test]
    fn check_agent_messages_follow_spec_branch_order() {
        assert_eq!(
            agent_message_kinds(render_check_agent_messages(1, 0, 0, 0)),
            vec![
                AgentMessageKind::RepairInstruction,
                AgentMessageKind::RepairInstruction,
                AgentMessageKind::RepairInstruction,
                AgentMessageKind::FixIssues,
            ]
        );
        assert_eq!(
            agent_message_kinds(render_check_agent_messages(0, 0, 0, 0)),
            vec![AgentMessageKind::AllClear]
        );
        assert_eq!(
            agent_message_kinds(render_check_agent_messages(0, 0, 1, 0)),
            vec![AgentMessageKind::CommitNotice]
        );
        assert_eq!(
            agent_message_kinds(render_check_agent_messages(1, 0, 2, 0)),
            vec![
                AgentMessageKind::CommitNotice,
                AgentMessageKind::RepairInstruction,
                AgentMessageKind::RepairInstruction,
                AgentMessageKind::RepairInstruction,
                AgentMessageKind::ThenFixRemaining,
            ]
        );
        assert_eq!(
            agent_message_kinds(render_check_agent_messages(0, 0, 1, 1)),
            vec![
                AgentMessageKind::RepairInstruction,
                AgentMessageKind::RepairInstruction,
                AgentMessageKind::RepairInstruction,
                AgentMessageKind::FixIssues,
            ]
        );
    }

    fn agent_message_kinds(messages: Vec<String>) -> Vec<AgentMessageKind> {
        messages
            .iter()
            .map(|message| {
                if message.starts_with('✓') {
                    AgentMessageKind::AllClear
                } else if message.contains("compared to HEAD") {
                    AgentMessageKind::CommitNotice
                } else if message.starts_with('❕') {
                    AgentMessageKind::RepairInstruction
                } else if message.starts_with("▷ Then") {
                    AgentMessageKind::ThenFixRemaining
                } else if message.starts_with('▷') {
                    AgentMessageKind::FixIssues
                } else {
                    panic!("unclassified agent message: {message}");
                }
            })
            .collect()
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
