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
// command form. The workflow reports a command error separately, preserves the
// outcome feedback specified by `emit_check_feedback`, and then appends the
// command-error next action so it is the last guidance shown to the user.
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
    let messages = completion_feedback_messages(
        report,
        feedback_context,
        failure_history_feedback,
        command_failed,
    );
    write_stdout_message_lines(output, messages, "check feedback").map_err(CommandError::from)
}

fn completion_feedback_messages(
    report: &CheckRunReport,
    feedback_context: CheckFeedbackContext,
    failure_history_feedback: Option<&crate::xpec_state::FailureHistoryFeedback>,
    command_failed: bool,
) -> Vec<String> {
    let mut messages = check_feedback_messages(report, feedback_context, failure_history_feedback);
    if command_failed {
        // [2Z,ex] `emit_check_feedback` still describes the recorded xpec
        // outcomes exactly. A later command failure gets its own final action,
        // preventing success or commit guidance from being the user's next
        // step while preserving both independently true facts.
        messages.extend(command_error_feedback_messages(feedback_context));
    }
    messages
}

pub(crate) fn check_feedback_messages(
    report: &CheckRunReport,
    feedback_context: CheckFeedbackContext,
    failure_history_feedback: Option<&crate::xpec_state::FailureHistoryFeedback>,
) -> Vec<String> {
    let issue_ids = report_issue_display_ids(report);
    // [2Z,3k,ex,KD,kK] Evaluation outcomes and command completion are
    // independent: a persistence or trailer failure does not rewrite result
    // records or their canonical feedback. The workflow emits a separate
    // command-error diagnostic before this outcome-only feedback. A sole
    // failed xpec enters the renderer's explicit history assertion.
    // [ex,kK] Violating this invariant deliberately panics; the outer check
    // lifecycle catches that panic, independently attempts every remaining
    // `finally` effect, and then resumes the original assertion panic.
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

    #[test] // xpec: ex
    #[should_panic(expected = "the current failure record must be appended to fail history")]
    fn canonical_history_assertion_rejects_an_unrecorded_single_failure() {
        let report = CheckRunReport {
            records: vec![failed_record("x")],
            cached_passes: Vec::new(),
            pending: 0,
        };
        let context = CheckFeedbackContext::from_tree_oids("checked", "head", "head");

        check_feedback_messages(&report, context, None);
    }

    #[test] // xpec: 2Z,ex
    fn command_failure_action_follows_canonical_outcome_feedback() {
        let report = CheckRunReport {
            records: Vec::new(),
            cached_passes: Vec::new(),
            pending: 0,
        };
        let context = CheckFeedbackContext::from_tree_oids("checked", "head", "head");

        let messages = completion_feedback_messages(&report, context, None, true);

        assert_eq!(
            messages,
            vec![
                "✓ All checks passed. Commit the staged changes!".to_string(),
                "▷ Fix the reported error and run `canon check` again!".to_string(),
            ]
        );
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
