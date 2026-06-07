use super::shared::write_stdout_record;
use crate::check::core::{for_each_unique_report_record, CheckRecord, CheckRunReport};
use std::io::Write;
use std::time::Duration;

const ALL_CHECKS_PASSED_MESSAGE: &str = "✓ All checks passed. Commit is allowed.";
const VERIFY_EVIDENCE_MESSAGE: &str =
    "❕ Verify that the evidence supports the observed answer and answers the expectation question; treat unsupported evidence as a readability issue.";
const PLAN_REPAIR_MESSAGE: &str =
    "❕ Plan the repair, then run `canon show -- <PATHSPEC>...` for the planned edit paths to identify expectations that may be affected.";
const USE_EXPECTATIONS_MESSAGE: &str =
    "❕ Use the matching expectations to avoid regressions while fixing the issues.";
const FIX_ISSUES_MESSAGE: &str = "▷ Fix the issues and run `canon check` again!";
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
    let SummaryOutcomeCounts {
        passed,
        failed,
        errors,
    } = summary_outcome_counts(report);
    let mut outcomes = Vec::new();
    if failed > 0 {
        outcomes.push(format!("{} failed", failed));
    }
    if errors > 0 {
        outcomes.push(format!(
            "{} {}",
            errors,
            if errors == 1 { "error" } else { "errors" }
        ));
    }
    if passed > 0 {
        outcomes.push(format!("{} passed", passed));
    }
    if report.skipped > 0 {
        outcomes.push(format!("{} skipped", report.skipped));
    }
    if outcomes.is_empty() {
        outcomes.push("0 passed".to_string());
    }
    let inner = format!(" {} in {:.2}s ", outcomes.join(", "), elapsed.as_secs_f64());
    format!("{}\n", pad_summary_line(&inner))
}

pub(crate) fn render_check_agent_messages(
    num_failed: usize,
    num_errors: usize,
    num_fixes: usize,
    num_regressions: usize,
) -> Vec<String> {
    let num_issues = num_failed + num_errors;
    if num_regressions > 0 || (num_issues > 0 && num_fixes == 0) {
        let mut messages = repair_instruction_messages();
        messages.push(FIX_ISSUES_MESSAGE.to_string());
        return messages;
    }
    if num_issues == 0 && num_fixes == 0 {
        return vec![ALL_CHECKS_PASSED_MESSAGE.to_string()];
    }

    let mut messages = vec![pass_improvement_notice(num_fixes).expect("positive fix count")];
    if num_issues > 0 {
        messages.extend(repair_instruction_messages());
        messages.push(THEN_FIX_REMAINING_MESSAGE.to_string());
    }
    messages
}

fn repair_instruction_messages() -> Vec<String> {
    vec![
        VERIFY_EVIDENCE_MESSAGE.to_string(),
        PLAN_REPAIR_MESSAGE.to_string(),
        USE_EXPECTATIONS_MESSAGE.to_string(),
    ]
}

fn pass_improvement_notice(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some(format!(
            "▷ +1 pass compared to HEAD. {}",
            PASS_IMPROVEMENT_COMMIT_SUFFIX
        )),
        count => Some(format!(
            "▷ +{} passes compared to HEAD. {}",
            count, PASS_IMPROVEMENT_COMMIT_SUFFIX
        )),
    }
}

pub(crate) struct SummaryOutcomeCounts {
    pub(crate) passed: usize,
    pub(crate) failed: usize,
    pub(crate) errors: usize,
}

pub(crate) fn summary_outcome_counts(report: &CheckRunReport) -> SummaryOutcomeCounts {
    let mut counts = SummaryOutcomeCounts {
        passed: 0,
        failed: 0,
        errors: 0,
    };
    for_each_unique_report_record(&report.records, &report.cached, |record| {
        add_summary_record(&mut counts, record)
    });
    counts
}

fn add_summary_record(counts: &mut SummaryOutcomeCounts, record: &CheckRecord) {
    if record.passed() {
        counts.passed += 1;
    } else if record.requires_human_review() {
        counts.errors += 1;
    } else {
        counts.failed += 1;
    }
}

fn pad_summary_line(inner: &str) -> String {
    const WIDTH: usize = 80;
    let width = WIDTH.max(inner.len() + 2);
    let padding = width - inner.len();
    let left = padding / 2;
    let right = padding - left;
    format!("{}{}{}", "=".repeat(left), inner, "=".repeat(right))
}
