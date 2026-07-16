use super::shared::write_stdout_record;
use crate::check::core::{for_each_unique_report_record, CheckRunReport};
use std::io::Write;
use std::time::Duration;

const ALL_CHECKS_PASSED_MESSAGE: &str = "✓ All checks passed. Commit is allowed.";
const VERIFY_EVIDENCE_MESSAGE: &str =
    "❕ Verify that the evidence supports the observed answer and answers the expectation question; treat unsupported evidence as a readability issue.";
const USE_EXPECTATIONS_MESSAGE: &str =
    "❕ Use the matching expectations to avoid regressions while fixing the issues.";
const FIX_ISSUES_MESSAGE: &str = "▷ Fix the issues and run `canon check` again!";
const CONTINUE_EVALUATION_MESSAGE: &str = "▷ Run `canon check` to continue evaluation.";
const THEN_FIX_REMAINING_MESSAGE: &str =
    "▷ Then fix the remaining issues and run `canon check` again!";
const PASS_IMPROVEMENT_COMMIT_SUFFIX: &str = "Commit the staged changes NOW!";

pub(crate) fn write_summary_line(
    result_output: &mut dyn Write,
    report: &CheckRunReport,
    elapsed: Duration,
) -> Result<(), String> {
    let line = render_check_summary(report, elapsed);
    write_stdout_record(result_output, line.as_bytes(), "check summary")
}

fn render_check_summary(report: &CheckRunReport, elapsed: Duration) -> String {
    let SummaryOutcomeCounts { passed, failed } = summary_outcome_counts(report);
    let mut outcomes = Vec::new();
    if failed > 0 {
        outcomes.push(format!("{} failed", failed));
    }
    if passed > 0 {
        outcomes.push(format!("{} passed", passed));
    }
    if report.skipped > 0 {
        outcomes.push(format!("{} pending", report.skipped));
    }
    if outcomes.is_empty() {
        outcomes.push("0 passed".to_string());
    }
    let inner = format!(" {} in {:.2}s ", outcomes.join(", "), elapsed.as_secs_f64());
    format!("{}\n", pad_summary_line(&inner))
}

pub(crate) fn render_check_agent_messages(
    failed: &[String],
    num_new_passes: usize,
    num_regressions: usize,
    num_pending: usize,
) -> Vec<String> {
    if num_regressions > 0 || (!failed.is_empty() && num_new_passes == 0) {
        let mut messages = repair_instruction_messages(failed);
        messages.push(FIX_ISSUES_MESSAGE.to_string());
        return messages;
    }
    if failed.is_empty() && num_pending > 0 {
        return vec![CONTINUE_EVALUATION_MESSAGE.to_string()];
    }
    if failed.is_empty() && num_new_passes == 0 {
        return vec![ALL_CHECKS_PASSED_MESSAGE.to_string()];
    }

    assert!(num_new_passes > 0);
    let mut messages =
        vec![pass_improvement_notice(num_new_passes).expect("positive new-pass count")];
    if !failed.is_empty() {
        messages.extend(repair_instruction_messages(failed));
        messages.push(THEN_FIX_REMAINING_MESSAGE.to_string());
    }
    messages
}

fn repair_instruction_messages(failed: &[String]) -> Vec<String> {
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

fn pass_improvement_notice(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some(format!("▷ +1 pass. {}", PASS_IMPROVEMENT_COMMIT_SUFFIX)),
        count => Some(format!(
            "▷ +{} passes. {}",
            count, PASS_IMPROVEMENT_COMMIT_SUFFIX
        )),
    }
}

pub(crate) struct SummaryOutcomeCounts {
    pub(crate) passed: usize,
    pub(crate) failed: usize,
}

pub(crate) fn summary_outcome_counts(report: &CheckRunReport) -> SummaryOutcomeCounts {
    let mut counts = SummaryOutcomeCounts {
        passed: 0,
        failed: 0,
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
    let width = WIDTH.max(inner.len() + 2);
    let padding = width - inner.len();
    let left = padding / 2;
    let right = padding - left;
    format!("{}{}{}", "=".repeat(left), inner, "=".repeat(right))
}
