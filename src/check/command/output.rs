use crate::check::core::types::{is_line_break_char, CheckRecord, CheckRunReport, ParsedAnswer};
use crate::json_util::compact_json_string_array;
use crate::logs::push_json_control_escape;
use crate::token_usage_types::TokenUsage;
use std::collections::BTreeSet;
use std::io::Write;
use std::time::Duration;

pub(crate) fn write_and_flush_result_output(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
) -> Result<(), String> {
    if let Some(writer) = result_output.as_mut() {
        let line = render_check_output_record(record);
        write_stdout_record(*writer, line.as_bytes(), "check result")?;
    }
    Ok(())
}

pub(crate) fn write_summary_line(
    result_output: &mut dyn Write,
    report: &CheckRunReport,
    elapsed: Duration,
) -> Result<(), String> {
    let line = render_check_summary(report, elapsed);
    write_stdout_record(result_output, line.as_bytes(), "check summary")
}

pub(crate) fn write_query_output(
    result_output: &mut dyn Write,
    answer: &ParsedAnswer,
) -> Result<(), String> {
    // Query output is intentionally separate from the selected-expectation
    // check output contract because query mode has no expectation selector,
    // expected answer, reusable history write, or final check summary.
    let output = render_query_output(answer);
    write_stdout_record(result_output, output.as_bytes(), "query result")
}

pub(crate) fn write_stdout_line_record(
    writer: &mut dyn Write,
    line: &str,
    description: &str,
) -> Result<(), String> {
    let mut output = String::with_capacity(line.len() + 1);
    output.push_str(line);
    output.push('\n');
    write_stdout_record(writer, output.as_bytes(), description)
}

fn write_stdout_record(
    writer: &mut dyn Write,
    bytes: &[u8],
    description: &str,
) -> Result<(), String> {
    writer
        .write_all(bytes)
        .map_err(|err| format!("failed to write {} to stdout: {}", description, err))?;
    writer
        .flush()
        .map_err(|err| format!("failed to flush {} to stdout: {}", description, err))
}

pub(crate) fn render_query_output(answer: &ParsedAnswer) -> String {
    let mut output = String::new();
    if let Some(error) = answer.error.as_deref() {
        output.push_str("Error: ");
        output.push_str(&escape_check_output_text(error));
    } else {
        output.push_str("Observed: ");
        output.push_str(&escape_check_output_text(&answer.answer));
    }
    output.push('\n');
    output.push_str("Evidence: ");
    output.push_str(&escape_check_output_text(&answer.evidence));
    output.push('\n');
    if let Some(suggestion) = answer.q_scope_suggestion.as_deref() {
        output.push_str("Suggested q-scope: ");
        output.push_str(&compact_json_string_array(suggestion));
        output.push('\n');
    }
    output
}

pub(crate) fn render_check_output_record(record: &CheckRecord) -> String {
    if record.passed() {
        return format!("{}. OK\n", record.display_id);
    }
    let is_error = record_requires_human_review(record);
    let status = if is_error { "ERROR" } else { "FAILED" };
    let mut output = String::new();
    output.push_str(&format!("{}. {}\n", record.display_id, status));
    // This is the spec's `<escaped question>` line, not an extra line beyond
    // the six-line failed and five-line error layouts.
    output.push_str(&escape_check_output_text(record.prompt_text()));
    output.push('\n');
    if is_error {
        output.push_str("Error: ");
        let error = record
            .review_error_text()
            .expect("error records must expose an error value");
        output.push_str(&escape_check_output_text(error));
        output.push('\n');
    } else {
        output.push_str("Expected: ");
        output.push_str(&escape_check_output_text(
            record.expected_text().unwrap_or(""),
        ));
        output.push('\n');
        output.push_str("Observed: ");
        output.push_str(&escape_check_output_text(&record.observed));
        output.push('\n');
    }
    output.push_str("Evidence: ");
    output.push_str(&escape_check_output_text(&record.evidence));
    output.push('\n');
    if !is_error {
        if let Some(suggestion) = record.suggested_q_scope.as_deref() {
            output.push_str("Suggested q-scope: ");
            output.push_str(&compact_json_string_array(suggestion));
            output.push('\n');
        }
    }
    output
}

pub(crate) fn render_token_usage_summary(usage: TokenUsage) -> String {
    format!(
        "Token usage: total={} input={} (+ {} cached) output={} (reasoning {})",
        usage.total_tokens,
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.output_tokens,
        usage.reasoning_output_tokens
    )
}

pub(crate) fn render_check_summary(report: &CheckRunReport, elapsed: Duration) -> String {
    // Summary order is fixed to match the spec and pytest-style labels:
    // failed, error/errors, passed, skipped.
    // Cached passes are part of the pass category even though they have no
    // per-expectation stdout line.
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
    // `skipped` is the count of expectations outside the summary categories.
    // Cached passes have no per-expectation stdout line, but they are still
    // pass results and therefore are not skipped in the summary.
    let skipped = report.skipped;
    if skipped > 0 {
        outcomes.push(format!("{} skipped", skipped));
    }
    if outcomes.is_empty() {
        outcomes.push("0 passed".to_string());
    }
    let inner = format!(" {} in {:.2}s ", outcomes.join(", "), elapsed.as_secs_f64());
    format!("{}\n", pad_summary_line(&inner))
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
    let mut seen = BTreeSet::new();
    for record in &report.records {
        if seen.insert(record.id.clone()) {
            add_summary_record(&mut counts, record);
        }
    }
    for cached in &report.cached {
        let id = if cached.record.id.is_empty() {
            &cached.expectation.id
        } else {
            &cached.record.id
        };
        if seen.insert(id.clone()) {
            add_summary_record(&mut counts, &cached.record);
        }
    }
    counts
}

fn add_summary_record(counts: &mut SummaryOutcomeCounts, record: &CheckRecord) {
    if record.passed() {
        counts.passed += 1;
    } else if record_requires_human_review(record) {
        counts.errors += 1;
    } else {
        counts.failed += 1;
    }
}

pub(crate) fn pad_summary_line(inner: &str) -> String {
    const WIDTH: usize = 80;
    // Reserve at least one `=` on each side even when the outcome text is
    // wider than the usual summary width.
    let width = WIDTH.max(inner.len() + 2);
    let padding = width - inner.len();
    let left = padding / 2;
    let right = padding - left;
    format!("{}{}{}", "=".repeat(left), inner, "=".repeat(right))
}

pub(crate) fn record_requires_human_review(record: &CheckRecord) -> bool {
    record.review_error_text().is_some()
}

pub(crate) fn escape_check_output_text(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        match ch {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if is_line_break_char(ch) || ch.is_control() => {
                push_check_output_unicode_escape(&mut output, ch);
            }
            ch => output.push(ch),
        }
    }
    output
}

fn push_check_output_unicode_escape(output: &mut String, ch: char) {
    if (ch as u32) <= 0xff {
        push_json_control_escape(output, ch as u8);
    } else {
        output.push_str(&format!("\\u{:04x}", ch as u32));
    }
}
