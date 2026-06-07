use super::escape::escape_check_output_text;
use super::shared::write_stdout_record;
use crate::check::core::CheckRecord;
use crate::json_util::compact_json_string_array;
use std::io::Write;

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

fn render_check_output_record(record: &CheckRecord) -> String {
    let mut output = format!("{}.", record.display_id);
    output.push_str(&render_check_output_record_completion(record));
    output
}

pub(super) fn render_check_output_record_completion(record: &CheckRecord) -> String {
    if record.passed() {
        return " OK\n".to_string();
    }
    let is_error = record_requires_human_review(record);
    let status = if is_error { "ERROR" } else { "FAILED" };
    let mut output = String::new();
    output.push_str(&format!(" {}\n", status));
    output.push_str(&escape_check_output_text(record.question_text()));
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
            record.expected_answer_text().unwrap_or(""),
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
        if let Some(suggestion) = record.question_scope_suggestion.as_deref() {
            output.push_str("Suggested q-scope: ");
            output.push_str(&compact_json_string_array(suggestion));
            output.push('\n');
        }
    }
    output
}

pub(crate) fn record_requires_human_review(record: &CheckRecord) -> bool {
    record.review_error_text().is_some()
}
