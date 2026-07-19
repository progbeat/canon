use super::shared::write_stdout_record;
use crate::check::core::{for_each_unique_report_record, CheckRunReport};
use std::io::Write;
use std::time::Duration;

const ALL_CHECKS_PASSED_MESSAGE: &str = "✓ All checks passed.";
const COMMIT_STAGED_CHANGES_MESSAGE: &str = "✓ All checks passed. Commit the staged changes!";
const VERIFY_EVIDENCE_MESSAGE: &str =
    "❕ Verify that the evidence supports the observed answer and answers the expectation question; treat unsupported evidence as a readability issue.";
const USE_EXPECTATIONS_MESSAGE: &str =
    "❕ Use the matching expectations to avoid regressions while fixing the issues.";
const FIX_ISSUES_MESSAGE: &str = "▷ Fix the issues and run `canon check` again!";
const CONTINUE_EVALUATION_MESSAGE: &str = "▷ Run `canon check` to continue evaluation.";

pub(crate) fn continue_evaluation_message() -> String {
    CONTINUE_EVALUATION_MESSAGE.to_string()
}

pub(crate) fn write_summary_line(
    result_output: &mut dyn Write,
    report: &CheckRunReport,
    elapsed: Duration,
) -> Result<(), String> {
    let line = render_check_summary(report, elapsed);
    write_stdout_record(result_output, line.as_bytes(), "check summary")
}

fn render_check_summary(report: &CheckRunReport, elapsed: Duration) -> String {
    let SummaryOutcomeCounts {
        failed,
        passed,
        pending,
    } = summary_outcome_counts(report);
    let mut outcomes = Vec::new();
    if failed > 0 {
        outcomes.push(format!("{} failed", failed));
    }
    if passed > 0 {
        outcomes.push(format!("{} passed", passed));
    }
    if pending > 0 {
        outcomes.push(format!("{} pending", pending));
    }
    if outcomes.is_empty() {
        outcomes.push("0 passed".to_string());
    }
    let inner = format!(" {} in {:.2}s ", outcomes.join(", "), elapsed.as_secs_f64());
    format!("{}\n", pad_summary_line(&inner))
}

pub(crate) fn render_check_agent_messages(
    failed: &[String],
    num_pending: usize,
    need_to_commit: bool,
) -> Vec<String> {
    if !failed.is_empty() {
        let mut messages = repair_instruction_messages(failed);
        messages.push(FIX_ISSUES_MESSAGE.to_string());
        return messages;
    }
    if num_pending > 0 {
        return vec![continue_evaluation_message()];
    }
    vec![if need_to_commit {
        COMMIT_STAGED_CHANGES_MESSAGE.to_string()
    } else {
        ALL_CHECKS_PASSED_MESSAGE.to_string()
    }]
}

fn repair_instruction_messages(failed: &[String]) -> Vec<String> {
    // xpec: 9b
    assert!(
        !failed.is_empty(),
        "repair instructions require at least one failed xpec"
    );
    vec![
        VERIFY_EVIDENCE_MESSAGE.to_string(),
        plan_repair_message(failed),
        USE_EXPECTATIONS_MESSAGE.to_string(),
    ]
}

fn plan_repair_message(failed: &[String]) -> String {
    // [AL] Mirror `_repair_instructions` literally: render one `not:<short ID>`
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

pub(crate) struct SummaryOutcomeCounts {
    pub(crate) failed: usize,
    pub(crate) passed: usize,
    pub(crate) pending: usize,
}

pub(crate) fn summary_outcome_counts(report: &CheckRunReport) -> SummaryOutcomeCounts {
    let mut counts = SummaryOutcomeCounts {
        failed: 0,
        passed: 0,
        pending: report.pending,
    };
    for_each_unique_report_record(&report.records, &report.cached, |record| {
        if record.passed() {
            counts.passed += 1;
        } else {
            counts.failed += 1;
        }
    });
    counts
}

fn pad_summary_line(inner: &str) -> String {
    const WIDTH: usize = 80;
    // [hJ] Keep at least two padding characters regardless of the preferred
    // width, so splitting the padding always leaves an `=` on both sides.
    let padding = WIDTH.saturating_sub(inner.len()).max(2);
    let left = padding / 2;
    let right = padding - left;
    format!("{}{}{}", "=".repeat(left), inner, "=".repeat(right))
}

#[cfg(test)]
mod tests {
    use super::pad_summary_line;

    #[test] // xpec: 7N
    fn long_summary_still_has_equals_padding_on_both_sides() {
        let inner = format!(" {} ", "long outcome ".repeat(10));

        let line = pad_summary_line(&inner);

        assert!(line.starts_with('='));
        assert!(line.ends_with('='));
        assert_eq!(line.len(), inner.len() + 2);
    }

    #[test] // xpec: hJ
    fn summary_one_short_of_preferred_width_has_equals_on_both_sides() {
        let inner = "x".repeat(79);

        let line = pad_summary_line(&inner);

        assert_eq!(line, format!("={inner}="));
    }
}
