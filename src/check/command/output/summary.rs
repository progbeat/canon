use super::shared::write_stdout_record;
use crate::check::core::{CheckRecord, CheckRunReport};
use std::collections::BTreeSet;
use std::io::Write;
use std::time::Duration;

const ALL_CHECKS_PASSED_MESSAGE: &str = "✓ All checks passed. Commit is allowed.";
const VERIFY_EVIDENCE_MESSAGE: &str =
    "❕ Verify that the evidence supports the observed answer and answers the expectation question; treat unsupported evidence as a readability issue.";
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
        blocked,
        passed,
        failed,
        errors,
    } = summary_outcome_counts(report);
    let mut outcomes = Vec::new();
    if blocked > 0 {
        outcomes.push(format!("{} blocked", blocked));
    }
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
    errors: &[String],
    num_new_passes: usize,
    num_regressions: usize,
    num_pending: usize,
) -> Vec<String> {
    let num_issues = failed.len() + errors.len();
    if num_regressions > 0 || (num_issues > 0 && num_new_passes == 0) {
        let mut messages = repair_instruction_messages(failed, errors);
        messages.push(FIX_ISSUES_MESSAGE.to_string());
        return messages;
    }
    if num_issues == 0 && num_new_passes == 0 {
        assert_eq!(num_pending, 0);
        return vec![ALL_CHECKS_PASSED_MESSAGE.to_string()];
    }

    assert!(num_new_passes > 0);
    let mut messages =
        vec![pass_improvement_notice(num_new_passes).expect("positive new-pass count")];
    if num_issues > 0 {
        messages.extend(repair_instruction_messages(failed, errors));
        messages.push(THEN_FIX_REMAINING_MESSAGE.to_string());
    } else {
        assert_eq!(num_pending, 0);
    }
    messages
}

fn repair_instruction_messages(failed: &[String], errors: &[String]) -> Vec<String> {
    vec![
        VERIFY_EVIDENCE_MESSAGE.to_string(),
        plan_repair_message(failed, errors),
        USE_EXPECTATIONS_MESSAGE.to_string(),
    ]
}

fn plan_repair_message(failed: &[String], errors: &[String]) -> String {
    let selectors = failed
        .iter()
        .chain(errors)
        .map(|id| format!("not:{id}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "❕ Plan the repair, then run `canon show {selectors} [not:<ALREADY_IN_CONTEXT_EXPECTATION>]... -- <PATHSPEC>...` for the planned edit paths to identify expectations that may be affected."
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
    pub(crate) blocked: usize,
    pub(crate) passed: usize,
    pub(crate) failed: usize,
    pub(crate) errors: usize,
}

pub(crate) fn summary_outcome_counts(report: &CheckRunReport) -> SummaryOutcomeCounts {
    let mut counts = SummaryOutcomeCounts {
        blocked: usize::from(report.blocked.is_some()),
        passed: 0,
        failed: 0,
        errors: 0,
    };
    let mut seen = BTreeSet::new();
    for record in &report.records {
        if seen.insert(record.id.clone()) {
            add_evaluated_summary_record(&mut counts, record);
        }
    }
    for cached in &report.cached {
        let id = if cached.record.id.is_empty() {
            &cached.expectation.id
        } else {
            &cached.record.id
        };
        if seen.insert(id.clone()) {
            add_cached_summary_record(&mut counts, &cached.record);
        }
    }
    counts
}

fn add_evaluated_summary_record(counts: &mut SummaryOutcomeCounts, record: &CheckRecord) {
    if record.passed() {
        counts.passed += 1;
    } else if record.requires_human_review() {
        counts.errors += 1;
    } else {
        counts.failed += 1;
    }
}

fn add_cached_summary_record(counts: &mut SummaryOutcomeCounts, record: &CheckRecord) {
    if record.passed() {
        counts.passed += 1;
    } else if record.requires_human_review() {
        // Human-review last-error records are not cache hits; cache selection
        // rejects them before cached output or summary accounting.
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
