use crate::xpec_state::FailureHistoryFeedback;

const ALL_CHECKS_PASSED_MESSAGE: &str = "✓ All checks passed.";
const COMMIT_STAGED_CHANGES_MESSAGE: &str = "✓ All checks passed. Commit the staged changes!";
const VERIFY_EVIDENCE_MESSAGE: &str =
    "❕ Verify that the evidence supports the observed answer and answers the expectation question; treat unsupported evidence as a readability issue.";
const USE_EXPECTATIONS_MESSAGE: &str =
    "❕ Use the matching expectations to avoid regressions while fixing the issues.";
const FIX_ISSUES_MESSAGE: &str = "▷ Fix the issues and run `canon check` again!";
const FIX_REPORTED_ERROR_MESSAGE: &str = "▷ Fix the reported error and run `canon check` again!";
const CONTINUE_EVALUATION_MESSAGE: &str = "▷ Run `canon check` to continue evaluation.";
const RECURRING_FAILURE_MESSAGE: &str =
    "❕ Repeated `canon check` runs keep failing on the same xpec. Do not run `canon check` again yet.";
const ADAPT_RECURRING_WORKFLOW_MESSAGE: &str =
    "❕ Each time this warning appears, determine why your workflow allowed the recurrence and adapt it to reduce the chance of another one.";
// [UZ,ex] This is post-evaluation CLI feedback addressed to the human reviewer.
// It asks that reviewer to emulate the evaluator as an independent check; it
// is never part of the instructions sent to an evaluator agent.
const HUMAN_REVIEWER_DISPROVE_RECURRING_FAILURE_MESSAGE: &str =
    "❕ Emulate the evaluator agent: independently try to disprove the expected answer. Generalize each finding and fix every supported violation.";
const RETRY_AFTER_JUSTIFICATION_MESSAGE: &str =
    "▷ Run `canon check` again only after you can independently justify the expected answer!";

pub(crate) fn continue_evaluation_message() -> String {
    CONTINUE_EVALUATION_MESSAGE.to_string()
}

pub(crate) fn command_error_feedback_messages(context: CheckFeedbackContext) -> Vec<String> {
    // [2Z,KD,w] A command can fail after every collected expectation passed.
    // Keep its next action distinct from the canonical outcome-count branches
    // so the summary remains accurate without emitting success/commit advice.
    context.assert_default_against_head();
    vec![FIX_REPORTED_ERROR_MESSAGE.to_string()]
}

#[derive(Clone, Copy)]
pub(crate) struct CheckFeedbackContext {
    against_tree_is_head: bool,
    need_to_commit: Option<bool>,
}

impl CheckFeedbackContext {
    pub(crate) fn for_default_against_head() -> CheckFeedbackContext {
        CheckFeedbackContext {
            against_tree_is_head: true,
            need_to_commit: None,
        }
    }

    pub(crate) fn from_tree_oids(
        checked_tree_oid: &str,
        against_tree_oid: &str,
        head_tree_oid: &str,
    ) -> CheckFeedbackContext {
        CheckFeedbackContext {
            against_tree_is_head: against_tree_oid == head_tree_oid,
            need_to_commit: Some(checked_tree_oid != against_tree_oid),
        }
    }

    pub(crate) fn assert_default_against_head(self) {
        // xpec: w,ex
        assert!(
            self.against_tree_is_head,
            "check feedback requires against-tree OID to equal HEAD tree OID"
        );
    }
}

// [#emit_check_feedback]
pub(crate) fn render_check_feedback_messages(
    failed: &[String],
    num_pending: usize,
    context: CheckFeedbackContext,
    failure_history_feedback: Option<&FailureHistoryFeedback>,
) -> Vec<String> {
    // The canonical assertion precedes every feedback branch. Keeping its
    // OID-derived proof in the required context prevents early failure paths
    // from bypassing it by passing only a precomputed commit flag.
    context.assert_default_against_head();
    if !failed.is_empty() {
        let mut messages = repair_instruction_messages(failed, failure_history_feedback);
        messages.push(FIX_ISSUES_MESSAGE.to_string());
        return messages;
    }
    if num_pending > 0 {
        return vec![continue_evaluation_message()];
    }
    let need_to_commit = context
        .need_to_commit
        .expect("success feedback requires resolved checked and against tree OIDs");
    vec![if need_to_commit {
        COMMIT_STAGED_CHANGES_MESSAGE.to_string()
    } else {
        ALL_CHECKS_PASSED_MESSAGE.to_string()
    }]
}

fn repair_instruction_messages(
    failed: &[String],
    failure_history_feedback: Option<&FailureHistoryFeedback>,
) -> Vec<String> {
    // xpec: w
    assert!(
        !failed.is_empty(),
        "repair instructions require at least one failed xpec"
    );
    let mut messages = vec![VERIFY_EVIDENCE_MESSAGE.to_string()];
    if failed.len() == 1 {
        // xpec: ex
        assert_eq!(
            failure_history_feedback.map(|feedback| feedback.short_id.as_str()),
            Some(failed[0].as_str()),
            "the current failure record must be appended to fail history"
        );
        let feedback = failure_history_feedback
            .expect("the explicit failure-history assertion established feedback");
        if feedback.recurring {
            messages.extend([
                RECURRING_FAILURE_MESSAGE.to_string(),
                ADAPT_RECURRING_WORKFLOW_MESSAGE.to_string(),
                HUMAN_REVIEWER_DISPROVE_RECURRING_FAILURE_MESSAGE.to_string(),
            ]);
            if let Some(diff_from_oid) = feedback.diff_from_oid.as_deref() {
                messages.push(format!(
                    "❕ Xpec `{}` targets the diff. Look for violations only in files listed by `git diff --cached --numstat {diff_from_oid}`.",
                    feedback.short_id
                ));
            }
            messages.push(RETRY_AFTER_JUSTIFICATION_MESSAGE.to_string());
            return messages;
        }
    }
    messages.extend([
        plan_repair_message(failed),
        USE_EXPECTATIONS_MESSAGE.to_string(),
    ]);
    messages
}

fn plan_repair_message(failed: &[String]) -> String {
    // [ex] Mirror `_repair_instructions` literally: render one `not:<short ID>`
    // selector for every failed xpec, with no placeholder selectors.
    let selectors = failed
        .iter()
        .map(|id| format!("not:{id}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "❕ Plan the repair, then run `canon show {selectors} -- <PATHSPEC>...` for the planned edit paths to identify expectations that may be affected."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: ex,w
    fn feedback_messages_cover_documented_actions() {
        let changed = feedback_context(true);
        let unchanged = feedback_context(false);
        let single_failure_history = FailureHistoryFeedback {
            short_id: "a".to_string(),
            recurring: false,
            diff_from_oid: None,
        };
        let repair_messages =
            render_check_feedback_messages(&issues(&["a", "b"]), 0, changed, None);
        assert!(has_action(&repair_messages, "Fix the issues"));
        assert!(has_action(&repair_messages, "canon show not:a not:b"));
        assert!(has_action(
            &render_check_feedback_messages(&[], 0, unchanged, None),
            "All checks passed"
        ));
        assert!(!has_action(
            &render_check_feedback_messages(&[], 0, unchanged, None),
            "Commit the staged changes"
        ));
        assert!(has_action(
            &render_check_feedback_messages(&[], 0, changed, None),
            "Commit the staged changes"
        ));
        assert!(has_action(
            &render_check_feedback_messages(&[], 1, changed, None),
            "Run `canon check`"
        ));
        assert!(!has_action(
            &render_check_feedback_messages(
                &issues(&["a"]),
                0,
                changed,
                Some(&single_failure_history),
            ),
            "Commit the staged changes"
        ));
        assert!(has_action(
            &command_error_feedback_messages(changed),
            "Fix the reported error"
        ));
        assert!(has_action(
            &command_error_feedback_messages(CheckFeedbackContext::for_default_against_head()),
            "Fix the reported error"
        ));
    }

    #[test] // xpec: ex
    fn recurring_diff_failure_replaces_general_repair_steps() {
        let recurring = FailureHistoryFeedback {
            short_id: "x".to_string(),
            recurring: true,
            diff_from_oid: Some("abc123".to_string()),
        };

        let messages = render_check_feedback_messages(
            &issues(&["x"]),
            0,
            feedback_context(false),
            Some(&recurring),
        );

        assert!(has_action(&messages, "keep failing on the same xpec"));
        assert!(has_action(&messages, "git diff --cached --numstat abc123"));
        assert!(has_action(
            &messages,
            "only after you can independently justify"
        ));
        assert!(!has_action(&messages, "canon show not:x"));
        assert!(has_action(&messages, "Fix the issues"));
    }

    fn issues(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    fn has_action(messages: &[String], action: &str) -> bool {
        messages.iter().any(|message| message.contains(action))
    }

    fn feedback_context(need_to_commit: bool) -> CheckFeedbackContext {
        if need_to_commit {
            CheckFeedbackContext::from_tree_oids("checked", "head", "head")
        } else {
            CheckFeedbackContext::from_tree_oids("head", "head", "head")
        }
    }
}
