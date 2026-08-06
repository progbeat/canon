use crate::check::command::output::{
    command_error_feedback_messages, render_check_feedback_messages, write_stdout_message_lines,
    CheckFeedbackContext,
};
use crate::check::core::{for_each_unique_report_record, CheckRunReport};
use crate::check::interrogation::write_check_lifecycle_finish_event;
use crate::cli::CommandError;
use std::io::Write;

// This module is deliberately not the public check-output renderer. The
// per-expectation stdout records and summary line live in `command::output`,
// token usage stderr output lives in `command::completion::usage`, and
// `command::workflow` orchestrates their order before calling
// `finish_check_report`. This module owns only the post-summary feedback plus
// finish logging. Success and error reports share both when allowed by the
// command form. The optional error changes the finish log payload and replaces
// success/commit guidance with command-error feedback for an otherwise
// all-passed report; the workflow caller owns the final command result.
pub(crate) struct CheckReportFinishContext<'b> {
    pub(crate) diagnostic_log: Option<&'b mut crate::logs::DiagnosticLogWriter>,
    pub(crate) result_output: &'b mut dyn Write,
    pub(crate) feedback_context: Option<CheckFeedbackContext>,
    pub(crate) failure_history_feedback: Option<&'b crate::xpec_state::FailureHistoryFeedback>,
}

pub(crate) fn finish_check_report(
    context: CheckReportFinishContext<'_>,
    report: &CheckRunReport,
    error: Option<&str>,
) -> Result<(), CommandError> {
    // The writers for per-expectation output and both public trailer parts have
    // already attempted to write and flush their output before this step. A
    // writer may have reported a write or flush failure, but this step neither
    // retries nor buffers those earlier pieces; it only attempts the agent
    // feedback and finish lifecycle log.
    let mut post_finish_error = None;
    let mut finish_error = error.map(str::to_string);
    if let Some(feedback_context) = context.feedback_context {
        if let Err(err) = write_check_feedback(
            report,
            context.result_output,
            feedback_context,
            context.failure_history_feedback,
            error.is_some(),
        ) {
            finish_error.get_or_insert_with(|| err.to_string());
            post_finish_error.get_or_insert(err);
        }
    }
    if let Some(diagnostic_log) = context.diagnostic_log {
        write_check_lifecycle_finish_event(diagnostic_log, false, finish_error.as_deref())?;
    }
    if let Some(err) = post_finish_error {
        return Err(err);
    }
    Ok(())
}

fn write_check_feedback(
    report: &CheckRunReport,
    output: &mut dyn Write,
    feedback_context: CheckFeedbackContext,
    failure_history_feedback: Option<&crate::xpec_state::FailureHistoryFeedback>,
    command_failed: bool,
) -> Result<(), CommandError> {
    let messages = check_feedback_messages(
        report,
        feedback_context,
        failure_history_feedback,
        command_failed,
    );
    write_stdout_message_lines(output, messages, "check feedback").map_err(CommandError::from)
}

pub(crate) fn check_feedback_messages(
    report: &CheckRunReport,
    feedback_context: CheckFeedbackContext,
    failure_history_feedback: Option<&crate::xpec_state::FailureHistoryFeedback>,
    command_failed: bool,
) -> Vec<String> {
    let issue_ids = report_issue_display_ids(report);
    // [2Z,3k,ex,KD,w] Evaluation outcomes and command completion are
    // independent: a persistence or trailer failure does not rewrite result
    // records. A sole failed xpec requires its just-appended history entry for
    // canonical repair feedback, though, so a failed history append routes to
    // recoverable command-error guidance instead of entering that assertion.
    let missing_required_failure_history =
        issue_ids.len() == 1 && failure_history_feedback.is_none();
    if command_failed
        && ((issue_ids.is_empty() && report.pending == 0) || missing_required_failure_history)
    {
        return command_error_feedback_messages(feedback_context);
    }
    render_check_feedback_messages(
        &issue_ids,
        report.pending,
        feedback_context,
        failure_history_feedback,
    )
}

fn report_issue_display_ids(report: &CheckRunReport) -> Vec<String> {
    let mut issue_ids = Vec::new();
    for_each_unique_report_record(&report.records, &report.cached_passes, |record| {
        if !record.passed() {
            issue_ids.push(record.display_id.clone());
        }
    });
    issue_ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::{CheckRecord, CheckResult};

    #[test] // xpec: 2Z,3k,ex
    fn failed_history_persistence_uses_recoverable_command_error_feedback() {
        let report = CheckRunReport {
            records: vec![failed_record("x")],
            cached_passes: Vec::new(),
            pending: 0,
        };
        let context = CheckFeedbackContext::from_tree_oids("checked", "head", "head");

        let messages = check_feedback_messages(&report, context, None, true);

        assert!(messages
            .iter()
            .any(|message| message.contains("Fix the reported error")));
        assert!(!messages
            .iter()
            .any(|message| message.contains("canon show not:x")));
    }

    fn failed_record(display_id: &str) -> CheckRecord {
        CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            result: CheckResult::Fail,
            to: crate::config_types::ExpectationTo::Agent,
            question: Some("Does it pass?".to_string()),
            expected_answer: Some("yes".to_string()),
            observed: "no".to_string(),
            error: None,
            evidence: Some("test evidence".to_string()),
            scope: vec!["src".to_string()],
            q_scope_suggestion: None,
            visible_tree_oid: Some("visible".to_string()),
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
            id: "11111111111111111111".to_string(),
            display_id: display_id.to_string(),
        }
    }
}
