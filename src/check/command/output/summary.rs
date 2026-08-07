use super::shared::write_stdout_record;
use crate::check::core::{for_each_unique_report_record, CheckRunReport};
use std::io::Write;
use std::time::Duration;

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
    let outcome_text = format!(" {} in {:.2}s ", outcomes.join(", "), elapsed.as_secs_f64());
    format!("{}\n", pad_summary_line(&outcome_text))
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
    for_each_unique_report_record(&report.records, &report.cached_passes, |record| {
        if record.passed() {
            counts.passed += 1;
        } else {
            counts.failed += 1;
        }
    });
    counts
}

fn pad_summary_line(outcome_text: &str) -> String {
    const PREFERRED_SUMMARY_LINE_WIDTH: usize = 80;
    // [kK] The summary format depicts variable `=` padding around the outcome
    // text, not literal run lengths. Prefer an 80-column line and keep at least
    // one padding character on each side when the outcome text is longer.
    let total_padding_width = PREFERRED_SUMMARY_LINE_WIDTH
        .saturating_sub(outcome_text.len())
        .max(2);
    let left_padding_width = total_padding_width / 2;
    let right_padding_width = total_padding_width - left_padding_width;
    format!(
        "{}{}{}",
        "=".repeat(left_padding_width),
        outcome_text,
        "=".repeat(right_padding_width)
    )
}
