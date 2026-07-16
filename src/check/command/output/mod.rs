// Command output is one component with a small facade. The leaf modules split
// stable stdout/stderr surfaces by output kind while keeping callers away from
// renderer internals.
// `canon check` uses this facade instead of `crate::output` because live
// progress shares stdout across threads. Hook communication, expectation
// result entries, token usage, and summary output are split across the command
// execution layer plus `record`, `usage`, and `summary`;
// every exported writer flushes through `shared::write_stdout_record` or
// `SharedCheckOutput` as soon as a record, token line, summary, query answer,
// agent message, or progress dot is eligible.
// `canon gate` is outside this check-output component because it has no live
// result/progress stream. Its user-visible lines are emitted through
// `crate::output::write_stderr_line` at the validation, error, or regression
// decision points where those lines first become eligible.
mod escape;
mod query;
mod record;
mod shared;
mod summary;
mod usage;

pub(crate) use escape::escape_check_output_text;
pub(crate) use query::finish_query_output;
pub(crate) use record::{
    start_expectation_report_output, start_query_report_output,
    write_result_output_without_started_report, StartedExpectationReportOutput,
};
pub(crate) use shared::{write_stdout_record, SharedCheckOutput};
pub(crate) use summary::{render_check_agent_messages, summary_outcome_counts, write_summary_line};
pub(crate) use usage::render_token_usage_summary;

#[cfg(test)]
// These tests are colocated with the command-output implementation. Dedicated
// tests outside implementation files exercise public CLI behavior instead.
mod tests {
    use super::{
        finish_query_output, render_check_agent_messages, render_token_usage_summary,
        start_expectation_report_output, start_query_report_output, summary_outcome_counts,
        write_result_output_without_started_report, write_summary_line, SharedCheckOutput,
    };
    use crate::check::core::{
        CachedExpectation, CheckRecord, CheckResult, CheckRunReport, ParsedAnswer,
        ERROR_INVALID_QUESTION,
    };
    use crate::check::ResolvedExpectation;
    use crate::config_types::AgentConfig;
    use crate::token_usage_types::TokenUsage;
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
    fn non_live_report_result_output_matches_documented_record_shape() {
        let mut bytes = Vec::new();
        let mut result_output = Some(&mut bytes as &mut dyn Write);

        write_result_output_without_started_report(&mut result_output, &passing_record()).unwrap();

        let rendered = String::from_utf8(bytes).unwrap();
        assert_result_entry(&rendered, "PASS");
    }

    #[test] // xpec: AL
    fn summary_and_token_usage_output_match_documented_lines() {
        let report = CheckRunReport {
            records: vec![passing_record()],
            cached: Vec::new(),
            skipped: 2,
        };
        let mut summary_bytes = Vec::new();

        write_summary_line(&mut summary_bytes, &report, Duration::from_millis(1250)).unwrap();

        let summary = String::from_utf8(summary_bytes).unwrap();
        assert!(summary.contains(" 1 passed, 2 pending in 1.25s "));
        assert!(summary.starts_with('='));
        assert!(summary.ends_with("=\n"));

        let usage = TokenUsage {
            total_tokens: 9,
            input_tokens: 4,
            cached_input_tokens: 3,
            output_tokens: 2,
            reasoning_output_tokens: 1,
        };
        assert_eq!(
            render_token_usage_summary(usage),
            "Token usage: total=9 input=4 (+ 3 cached) output=2 (reasoning 1)"
        );
    }

    #[test] // xpec: AL,8s
    fn failed_result_output_matches_documented_detail_lines() {
        let mut bytes = Vec::new();
        let mut result_output = Some(&mut bytes as &mut dyn Write);
        let mut record = failed_record();
        record.diff_from = Some(":against-tree".to_string());
        record.diff_from_tree_oid = Some("1234567890abcdef1234567890abcdef12345678".to_string());
        record.diff_from_tree_oid_abbrev = Some("1234567".to_string());

        write_result_output_without_started_report(&mut result_output, &record).unwrap();

        let rendered = String::from_utf8(bytes).unwrap();
        assert_result_entry(&rendered, "FAIL");
        assert!(rendered.contains("Does it pass?\n"));
        assert!(rendered.contains("diff-from: 1234567 (:against-tree)\n"));
        assert!(rendered.contains("expected: yes\n"));
        assert!(rendered.contains("observed: no\n"));
        assert!(rendered.contains("evidence: test evidence\n"));
    }

    #[test] // xpec: AL
    fn error_result_output_matches_documented_detail_lines() {
        let mut bytes = Vec::new();
        let mut result_output = Some(&mut bytes as &mut dyn Write);
        let mut record = review_record_with_id("11111111111111111111", "j");
        record.question_scope_suggestion = Some(vec!["src/check".to_string()]);
        record.diff_from = Some(":checkpoint".to_string());
        record.diff_from_tree_oid = Some("abcdef1234567890abcdef1234567890abcdef12".to_string());
        record.diff_from_tree_oid_abbrev = Some("abcdef1".to_string());

        write_result_output_without_started_report(&mut result_output, &record).unwrap();

        let rendered = String::from_utf8(bytes).unwrap();
        assert_result_entry(&rendered, "FAIL");
        assert!(rendered.contains("error: InvalidQuestion\n"));
        assert!(!rendered.contains("expected:"));
        assert!(!rendered.contains("observed:"));
        assert!(!rendered.contains("evidence:"));
    }

    #[test] // xpec: AL
    fn live_report_result_output_matches_documented_record_shape() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = SharedCheckOutput::new(Box::new(CapturedOutput {
            bytes: bytes.clone(),
        }));

        let report = start_expectation_report_output(output, "j");
        let finished = report.finish_with_record(&passing_record());
        assert!(!finished.needs_stderr_completion_notice());

        let completed = captured_string(&bytes);
        assert_result_entry(&completed, "PASS");
    }

    #[test] // xpec: sy,1h
    fn live_report_flushes_short_id_before_first_progress_marker() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = SharedCheckOutput::new(Box::new(CapturedOutput {
            bytes: bytes.clone(),
        }));

        let report = start_expectation_report_output(output, "j");

        assert_eq!(captured_string(&bytes), "j");
        let finished = report.finish_with_record(&passing_record());
        assert!(!finished.needs_stderr_completion_notice());
    }

    #[test] // xpec: 0N
    fn query_output_starts_with_progress_timeline_line() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = SharedCheckOutput::new(Box::new(CapturedOutput {
            bytes: bytes.clone(),
        }));
        let report = start_query_report_output(output);
        let answer = ParsedAnswer::answer(
            "yes".to_string(),
            "test evidence".to_string(),
            Some(vec!["src/check".to_string()]),
        );

        finish_query_output(report, &answer).unwrap();

        assert_eq!(
            captured_string(&bytes),
            ".\nobserved: yes\nevidence: test evidence\nq-scope-suggestion: [\"src/check\"]\n"
        );
    }

    #[test]
    fn agent_messages_cover_documented_actions() {
        assert!(has_action(
            &render_check_agent_messages(&issues(&["a"]), 0, 0, 0),
            "Fix the issues"
        ));
        assert!(has_action(
            &render_check_agent_messages(&[], 0, 0, 0),
            "All checks passed"
        ));
        assert!(has_action(
            &render_check_agent_messages(&[], 1, 0, 0),
            "Commit the staged changes"
        ));
        assert!(has_action(
            &render_check_agent_messages(&issues(&["a"]), 2, 0, 1),
            "Then fix the remaining issues"
        ));
        assert!(!has_action(
            &render_check_agent_messages(&issues(&["a"]), 1, 1, 0),
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

    fn passing_record_with_id(id: &str, display_id: &str) -> CheckRecord {
        record_with_identity(CheckResult::Pass, "yes", None, id, display_id)
    }

    fn review_record_with_id(id: &str, display_id: &str) -> CheckRecord {
        record_with_identity(
            CheckResult::Fail,
            "",
            Some(ERROR_INVALID_QUESTION),
            id,
            display_id,
        )
    }

    fn cached_expectation(record: CheckRecord) -> CachedExpectation {
        CachedExpectation {
            expectation: ResolvedExpectation {
                number: record.number,
                id: record.id.clone(),
                display_id: record.display_id.clone(),
                to: crate::config_types::ExpectationTo::Agent,
                rank: 0,
                question: record.question_text().to_string(),
                expected_answer: record.expected_answer_text().unwrap_or("yes").to_string(),
                question_context: String::new(),
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
            to: crate::config_types::ExpectationTo::Agent,
            question: Some("Does it pass?".to_string()),
            expected_answer: Some("yes".to_string()),
            observed: observed.to_string(),
            error: error.map(str::to_string),
            evidence: "test evidence".to_string(),
            scope: vec![".".to_string()],
            question_scope_suggestion: None,
            visible_tree_oid: "visible".to_string(),
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
            id: id.to_string(),
            display_id: display_id.to_string(),
        }
    }
}
