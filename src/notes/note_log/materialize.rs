use super::file::read_note_data;
use super::parse::find_note_log;
use super::record::NoteRecord;
use super::storage::encode_note_body_for_storage;
use crate::fs_util::{crossed_size_compaction_bucket, reject_symlink};
use crate::notes::header::{header, initial_content, normalize_body};
use crate::notes::index::write_file_atomically;
use crate::project_types::Note;
use std::fs;

const NOTE_LOG_COMPACT_MIN_BYTES: u64 = 64 * 1024;

pub(super) fn write_compacted_note_record(note: &Note, record: NoteRecord) -> Result<(), String> {
    let content = compacted_note_content(note, None, record)?;
    write_file_atomically(&note.path, content.as_bytes())
}

pub(super) fn maybe_compact_note_log(note: &Note, previous_size: u64) -> Result<(), String> {
    reject_symlink(&note.path)?;
    let size = fs::metadata(&note.path)
        .map_err(|err| format!("failed to inspect {}: {}", note.path.display(), err))?
        .len();
    if !crossed_size_compaction_bucket(previous_size, size, NOTE_LOG_COMPACT_MIN_BYTES) {
        return Ok(());
    }
    let content = read_note_data(note, |path| fs::read_to_string(path))?;
    let Some((log_start, _)) = find_note_log(note, &content)? else {
        return Ok(());
    };
    let log_bytes = content.len().saturating_sub(log_start);
    // Compact only after the appended log is at least as large as the retained
    // materialized prefix, so the rewrite is amortized by accumulated appends.
    if log_bytes < log_start {
        return Ok(());
    }
    let compacted = materialize_note_content(note, &content)?;
    if compacted.len() < content.len() {
        write_file_atomically(&note.path, compacted.as_bytes())?;
    }
    Ok(())
}

// `write` records produce a compact replacement file. `append` records for an
// existing note are persisted by appending a small private log entry. Once the
// accumulated log is large enough to pay for a rewrite, it is compacted back
// into the visible note text.
fn compacted_note_content(
    note: &Note,
    current: Option<&str>,
    record: NoteRecord,
) -> Result<String, String> {
    let mut output = match current {
        Some(content) => materialize_note_content(note, content)?,
        None => String::new(),
    };
    apply_note_record(note, &mut output, record);
    Ok(output)
}

fn materialize_note_content(note: &Note, content: &str) -> Result<String, String> {
    let Some((log_start, records)) = find_note_log(note, content)? else {
        return Ok(content.to_string());
    };

    let mut output = content[..log_start].to_string();
    for record in records {
        apply_note_record(note, &mut output, record);
    }
    Ok(output)
}

fn apply_note_record(note: &Note, output: &mut String, record: NoteRecord) {
    match record {
        NoteRecord::Write { text } => {
            *output = replacement_note_content(note, &text);
        }
        NoteRecord::Append { timestamp, text } => {
            if output.is_empty() {
                *output = initial_content(&note.key, &note.hash);
            }
            append_note_section(output, timestamp, &text);
        }
    }
}

fn replacement_note_content(note: &Note, text: &str) -> String {
    format!(
        "{}{}\n",
        header(&note.key, &note.hash),
        encode_note_body_for_storage(text)
    )
}

fn append_note_section(output: &mut String, timestamp: u64, text: &str) {
    append_note_section_with_body(output, timestamp, &encode_note_body_for_storage(text));
}

pub(super) fn append_visible_note_section(output: &mut String, timestamp: u64, text: &str) {
    append_note_section_with_body(output, timestamp, &normalize_body(text));
}

fn append_note_section_with_body(output: &mut String, timestamp: u64, body: &str) {
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push('\n');
    output.push_str(&note_section(timestamp, body));
}

pub(super) fn note_section(timestamp: u64, body: &str) -> String {
    format!("## {}\n\n{}\n", timestamp, body)
}
