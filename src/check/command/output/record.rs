mod progress;

use super::escape::{
    escape_check_output_text, push_diff_from_line, push_error_and_evidence_lines,
    push_escaped_check_output_line, push_observed_and_evidence_lines,
};
use super::shared::write_stdout_record;
#[cfg(test)]
use super::shared::SharedCheckOutput;
use crate::check::core::CheckRecord;
use crate::config_types::ExpectationTo;
use crate::evaluator::PROGRESS_TIMELINE_MARKER_INTERVAL;
use std::io::Write;
use std::time::Duration;

pub(crate) use progress::{
    publish_expectation_report, start_query_report_output, LiveProgressOutput,
};

const CSI_SAVE_CURSOR: &str = "\u{1b}[s";
// [Eg] These exact byte sequences are the interactive caller-output protocol
// for its two consumers: a POSIX terminal and Codex's shell-output renderer.
// A terminal-control abstraction would reproduce the same bytes while hiding
// the renderer requirement that makes the final conceal/reset pair necessary.
// They are compile-time literals appended once to an already rendered result;
// this output path performs no repository, filesystem, parsing, or hashing work
// whose result could be reused.
const INTERACTIVE_CALLER_RESULT_REPLACEMENT_PREFIX: &str = "\u{1b}[u\u{1b}[0J\r\u{1b}[0m";
const INTERACTIVE_CALLER_RESULT_RENDERER_SAFE_LINE_END: &str = "\u{1b}[8m\n\u{1b}[0m";

pub(crate) struct ExpectationReportWriteOutcome {
    short_id_was_printed: bool,
    result_was_printed: bool,
    stdout_result_append_had_problem: bool,
}

impl ExpectationReportWriteOutcome {
    pub(super) fn new(
        short_id_was_printed: bool,
        result_was_printed: bool,
        stdout_result_append_had_problem: bool,
    ) -> ExpectationReportWriteOutcome {
        ExpectationReportWriteOutcome {
            short_id_was_printed,
            result_was_printed,
            stdout_result_append_had_problem,
        }
    }

    pub(crate) fn anything_was_reported(&self) -> bool {
        self.short_id_was_printed || self.result_was_printed
    }

    pub(crate) fn needs_stderr_result_notice(&self) -> bool {
        self.stdout_result_append_had_problem || !self.anything_was_reported()
    }
}

// Results that finish before a live evaluation report starts still have the
// required final progress marker.
pub(crate) fn write_result_output_without_started_report(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
) -> Result<(), String> {
    if let Some(writer) = result_output.as_mut() {
        let line = render_check_output_record_with_initial_marker_timeline(record);
        write_stdout_record(*writer, line.as_bytes(), "check result")?;
    }
    Ok(())
}

pub(crate) fn write_caller_result_output_with_elapsed_timeline(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
    elapsed: Duration,
) -> Result<(), String> {
    if let Some(writer) = result_output.as_mut() {
        let timeline = caller_progress_timeline(elapsed)?;
        let line = render_check_output_record_with_timeline(record, &timeline);
        write_stdout_record(*writer, line.as_bytes(), "check result")?;
    }
    Ok(())
}

pub(crate) fn render_caller_prompt(question: &str) -> String {
    let mut prompt = String::new();
    if crate::platform::process::interactive_check_terminal() {
        prompt.push_str(CSI_SAVE_CURSOR);
    }
    prompt.push_str(&escape_check_output_text(question));
    prompt.push(' ');
    prompt
}

pub(crate) fn write_caller_result_output(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
    elapsed: Duration,
) -> Result<(), String> {
    write_caller_result_output_with_interactivity(
        result_output,
        record,
        elapsed,
        crate::platform::process::interactive_check_terminal(),
    )
}

fn write_caller_result_output_with_interactivity(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
    elapsed: Duration,
    interactive: bool,
) -> Result<(), String> {
    if !interactive {
        return write_caller_result_output_with_elapsed_timeline(result_output, record, elapsed);
    }
    let Some(writer) = result_output.as_mut() else {
        return Ok(());
    };
    let timeline = caller_progress_timeline(elapsed)?;
    let suffix = render_check_output_record_status_and_details(record);
    let (status, details) = suffix
        .split_once('\n')
        .expect("caller result status has a newline");
    // [Eg] One byte stream must replace the saved prompt in a terminal and
    // remain unambiguous in Codex's shell renderer. The replacement prefix
    // restores the cursor and erases the terminal suffix. The renderer-safe
    // line end hides the suffix retained by renderers that do not emulate
    // erase-to-end-of-screen, then resets styling for the detail lines.
    // Terminal-only erasure cannot affect such a renderer, while padding would
    // require the unavailable display width of potentially Unicode text.
    let line = format!(
        "{INTERACTIVE_CALLER_RESULT_REPLACEMENT_PREFIX}{}{timeline}{status}{INTERACTIVE_CALLER_RESULT_RENDERER_SAFE_LINE_END}{details}",
        record.display_id,
    );
    write_stdout_record(*writer, line.as_bytes(), "interactive caller result")
}

// [2gZ] A caller evaluation waits for human input and starts no evaluator turn,
// so model-turn timeout and retry events cannot occur on this timeline. Its
// elapsed full and final partial minutes therefore use the fallback `.` marker.
fn caller_progress_timeline(elapsed: Duration) -> Result<String, String> {
    let completed_minutes = elapsed.as_secs() / PROGRESS_TIMELINE_MARKER_INTERVAL.as_secs();
    let marker_count = usize::try_from(completed_minutes.saturating_add(1))
        .map_err(|_| "progress timeline marker count exceeds platform limits".to_string())?;
    Ok(".".repeat(marker_count))
}

fn render_check_output_record_with_initial_marker_timeline(record: &CheckRecord) -> String {
    render_check_output_record_with_timeline(record, ".")
}

pub(super) fn render_check_output_record_with_timeline(
    record: &CheckRecord,
    timeline: &str,
) -> String {
    let mut output = record.display_id.clone();
    output.push_str(timeline);
    output.push_str(&render_check_output_record_status_and_details(record));
    output
}

pub(super) fn render_check_output_record_status_and_details(record: &CheckRecord) -> String {
    let expected = record
        .expected_answer_text()
        .filter(|expected| !expected.is_empty())
        .expect("canon check output requires a non-empty expected answer");
    if record.passed() {
        return " OK\n".to_string();
    }
    let mut output = String::new();
    output.push_str(" FAIL\n");
    if let Some(error) = record.error.as_deref() {
        push_error_and_evidence_lines(&mut output, error, record.evidence.as_deref());
        return output;
    }
    if record.to == ExpectationTo::Caller {
        push_escaped_check_output_line(&mut output, "expected", expected);
        return output;
    }
    if record.to == ExpectationTo::Shell {
        for line in record.evidence.as_deref().unwrap_or("").lines() {
            output.push_str("│ ");
            output.push_str(line);
            output.push('\n');
        }
        output.push_str(&format!(
            "Command exited with code {} (expected {}).\n",
            record.observed, expected
        ));
        return output;
    }
    output.push_str(&escape_check_output_text(record.question_text()));
    output.push('\n');
    // The `diff-from:` line is part of a wrong-answer result only when the
    // record came from a Git-backed interrogation with a resolved diff base.
    // Cached records reconstruct the same in-memory abbreviation before
    // reaching this renderer.
    if let (Some(diff_from), Some(diff_from_tree_oid_abbrev)) = (
        record.diff_from.as_deref(),
        record.diff_from_tree_oid_abbrev.as_deref(),
    ) {
        push_diff_from_line(&mut output, diff_from, diff_from_tree_oid_abbrev);
    }
    push_escaped_check_output_line(&mut output, "expected", expected);
    push_observed_and_evidence_lines(&mut output, &record.observed, record.evidence.as_deref());
    output
}

#[cfg(test)]
mod tests;
