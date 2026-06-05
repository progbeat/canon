use crate::fs_util::{crossed_size_compaction_bucket, reject_symlink};
use crate::notes::header::{
    header, initial_content, normalize_body, verify_note_key_from_first_line,
};
use crate::notes::index::write_file_atomically;
use crate::notes::restore::error_with_restore_context;
use crate::platform::open_file_for_append_without_following_symlink;
use crate::project_types::Note;
use crate::time::unix_timestamp;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

const LEGACY_NOTE_LOG_MARKER: &str = "<!-- canon log v1 -->";
const NOTE_LOG_MARKER_PREFIX: &str = "<!-- canon log v1 ";
const NOTE_LOG_MARKER_SUFFIX: &str = " -->";
const NOTE_LOG_COMPACT_MIN_BYTES: u64 = 64 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(tag = "op")]
enum NoteRecord {
    #[serde(rename = "write")]
    Write { text: String },
    #[serde(rename = "append")]
    Append { timestamp: u64, text: String },
}

pub(crate) enum NoteTextOperation {
    Write,
    Append,
}

pub(crate) fn record_note_text(
    note: &Note,
    existed: bool,
    text: &str,
    operation: NoteTextOperation,
) -> Result<(), String> {
    let text = normalize_body(text);
    let record = match operation {
        NoteTextOperation::Write => NoteRecord::Write { text },
        NoteTextOperation::Append => NoteRecord::Append {
            timestamp: unix_timestamp()?,
            text,
        },
    };
    append_note_record(note, existed, record)
}

fn append_note_record(note: &Note, existed: bool, record: NoteRecord) -> Result<(), String> {
    match record {
        NoteRecord::Append { timestamp, text } if existed => {
            let previous_size =
                append_note_log_record(note, &NoteRecord::Append { timestamp, text })?;
            // The append is durable once the log record is flushed and synced.
            // Compaction is opportunistic; reporting its failure would invite
            // retrying an already-persisted append and duplicating visible
            // note content.
            let _ = maybe_compact_note_log(note, previous_size);
        }
        record => {
            // Replacement writes and first appends both persist the complete
            // note body; existing-note appends take the amortized log path.
            write_compacted_note_record(note, record)?;
        }
    }
    Ok(())
}

fn append_note_log_record(note: &Note, record: &NoteRecord) -> Result<u64, String> {
    let path = &note.path;
    let mut file = open_file_for_append_without_following_symlink(path)?;
    let previous_size = file
        .metadata()
        .map(|metadata| metadata.len())
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
    let mut line = serde_json::to_string(record).map_err(|err| {
        format!(
            "failed to encode note record for {}: {}",
            path.display(),
            err
        )
    })?;
    line.push('\n');
    // The caller holds NoteLock. Assemble one logical entry before appending
    // so canon's own writes never split a marker from its JSON record.
    let mut entry = String::new();
    entry.push('\n');
    entry.push_str(&note_log_marker(note, previous_size + 1));
    entry.push('\n');
    entry.push_str(&line);
    let append_result = file
        .write_all(entry.as_bytes())
        .and_then(|()| flush_and_sync_file(&mut file));
    if let Err(err) = append_result {
        return Err(error_with_restore_context(
            format!("failed to append {}: {}", path.display(), err),
            rollback_note_log_append(path, file, previous_size),
        ));
    }
    Ok(previous_size)
}

fn flush_and_sync_file(file: &mut fs::File) -> io::Result<()> {
    file.flush()?;
    file.sync_data()
}

fn rollback_note_log_append(
    path: &Path,
    mut file: fs::File,
    previous_size: u64,
) -> Result<(), String> {
    let err = match truncate_note_log_append(&mut file, previous_size) {
        Ok(()) => return Ok(()),
        Err(err) => err,
    };
    #[cfg(not(unix))]
    if err.kind() == io::ErrorKind::PermissionDenied {
        drop(file);
        let mut file = open_note_for_rollback(path)?;
        return truncate_note_log_append(&mut file, previous_size)
            .map_err(|err| rollback_note_log_append_error(path, previous_size, err));
    }
    Err(rollback_note_log_append_error(path, previous_size, err))
}

fn truncate_note_log_append(file: &mut fs::File, previous_size: u64) -> io::Result<()> {
    file.set_len(previous_size)
        .and_then(|()| flush_and_sync_file(file))
}

fn rollback_note_log_append_error(path: &Path, previous_size: u64, err: io::Error) -> String {
    format!(
        "failed to roll back {} to {} bytes after append failure: {}",
        path.display(),
        previous_size,
        err
    )
}

#[cfg(not(unix))]
fn open_note_for_rollback(path: &Path) -> Result<fs::File, String> {
    reject_symlink(path)?;
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|err| format!("failed to open {} for rollback: {}", path.display(), err))
}

#[cfg(test)]
pub(crate) fn rollback_note_log_append_for_test(
    path: &Path,
    previous_size: u64,
) -> Result<(), String> {
    let file = open_file_for_append_without_following_symlink(path)?;
    rollback_note_log_append(path, file, previous_size)
}

fn write_compacted_note_record(note: &Note, record: NoteRecord) -> Result<(), String> {
    let content = compacted_note_content(note, None, record)?;
    write_file_atomically(&note.path, content.as_bytes())
}

fn maybe_compact_note_log(note: &Note, previous_size: u64) -> Result<(), String> {
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

fn append_visible_note_section(output: &mut String, timestamp: u64, text: &str) {
    append_note_section_with_body(output, timestamp, &normalize_body(text));
}

fn append_note_section_with_body(output: &mut String, timestamp: u64, body: &str) {
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push('\n');
    output.push_str(&note_section(timestamp, body));
}

fn note_section(timestamp: u64, body: &str) -> String {
    format!("## {}\n\n{}\n", timestamp, body)
}

fn encode_note_body_for_storage(text: &str) -> String {
    normalize_body(text)
        .split('\n')
        .map(encode_note_storage_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn encode_note_storage_line(line: &str) -> String {
    if is_log_marker_family(line) {
        format!("\\{}", line)
    } else {
        line.to_string()
    }
}

fn decode_note_storage_line(line: &str) -> String {
    let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
    let suffix = &line[trimmed.len()..];
    if trimmed.strip_prefix('\\').is_some_and(is_log_marker_family) {
        format!("{}{}", &trimmed[1..], suffix)
    } else {
        line.to_string()
    }
}

fn is_log_marker_family(line: &str) -> bool {
    let line = line.trim_start_matches('\\');
    line == LEGACY_NOTE_LOG_MARKER
        || (line.starts_with(NOTE_LOG_MARKER_PREFIX) && line.ends_with(NOTE_LOG_MARKER_SUFFIX))
}

fn note_log_marker(note: &Note, marker_offset: u64) -> String {
    format!(
        "{}hash={} offset={}{}",
        NOTE_LOG_MARKER_PREFIX, note.hash, marker_offset, NOTE_LOG_MARKER_SUFFIX
    )
}

fn is_note_log_marker(note: &Note, line: &str, line_start: usize) -> bool {
    let Some((hash, offset)) = parse_note_log_marker(line) else {
        return false;
    };
    hash == note.hash && offset == line_start as u64
}

fn parse_note_log_marker(line: &str) -> Option<(&str, u64)> {
    let content = line
        .strip_prefix(NOTE_LOG_MARKER_PREFIX)?
        .strip_suffix(NOTE_LOG_MARKER_SUFFIX)?;
    let mut hash = None;
    let mut offset = None;
    for part in content.split_ascii_whitespace() {
        if let Some(value) = part.strip_prefix("hash=") {
            hash = Some(value);
        } else if let Some(value) = part.strip_prefix("offset=") {
            offset = value.parse::<u64>().ok();
        } else {
            return None;
        }
    }
    Some((hash?, offset?))
}

pub(crate) fn read_note_content(
    note: &Note,
    write: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    let reader = open_note_reader(note)?;
    stream_note_content(note, reader, write)
}

fn open_note_reader(note: &Note) -> Result<BufReader<fs::File>, String> {
    reject_symlink(&note.path)?;
    let file = fs::File::open(&note.path).map_err(|err| note_read_error(note, err))?;
    Ok(BufReader::new(file))
}

fn stream_note_content(
    note: &Note,
    mut reader: impl BufRead,
    mut write: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    let mut offset = 0;
    let Some((_, first_line)) = read_note_storage_line(note, &mut reader, &mut offset)? else {
        return verify_note_key_from_first_line(&note.path, "", &note.key);
    };
    verify_note_key_from_first_line(
        &note.path,
        first_line.trim_end_matches(&['\r', '\n'][..]),
        &note.key,
    )?;
    write(&first_line)?;

    loop {
        let Some((line_start, line)) = read_note_storage_line(note, &mut reader, &mut offset)?
        else {
            return Ok(());
        };
        let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
        if is_note_log_marker(note, trimmed, line_start) {
            stream_note_log(note, &mut reader, &mut write, &mut offset)?;
            return Ok(());
        }
        if trimmed == LEGACY_NOTE_LOG_MARKER {
            stream_legacy_note_log_or_text(note, &mut reader, &mut write, &mut offset, &line)?;
            return Ok(());
        }
        write(&decode_note_storage_line(&line))?;
    }
}

fn stream_note_log(
    note: &Note,
    reader: &mut impl BufRead,
    write: &mut impl FnMut(&str) -> Result<(), String>,
    offset: &mut usize,
) -> Result<(), String> {
    let mut first_record = true;
    loop {
        let Some((line_start, line)) = read_note_storage_line(note, reader, offset)? else {
            return Ok(());
        };
        match parse_note_log_line(NoteLogLineKind::Current { note, line_start }, &line) {
            ParsedNoteLogLine::Skip | ParsedNoteLogLine::Eof => continue,
            ParsedNoteLogLine::Unfinished => return Ok(()),
            ParsedNoteLogLine::Malformed(err) => {
                return Err(format!(
                    "malformed note log record in {}: {}",
                    note.path.display(),
                    err
                ));
            }
            ParsedNoteLogLine::Record(record) => {
                stream_note_record(note, record, write, first_record)?;
                first_record = false;
            }
        }
    }
}

fn stream_note_record(
    note: &Note,
    record: NoteRecord,
    write: &mut impl FnMut(&str) -> Result<(), String>,
    follows_log_separator: bool,
) -> Result<(), String> {
    match record {
        NoteRecord::Append { timestamp, text } => {
            let section = if follows_log_separator {
                note_section(timestamp, &normalize_body(&text))
            } else {
                let mut section = String::new();
                append_visible_note_section(&mut section, timestamp, &text);
                section
            };
            write(&section)
        }
        NoteRecord::Write { .. } => Err(format!(
            "malformed note log record in {}: write record cannot be streamed",
            note.path.display()
        )),
    }
}

fn stream_legacy_note_log_or_text(
    note: &Note,
    reader: &mut impl BufRead,
    write: &mut impl FnMut(&str) -> Result<(), String>,
    offset: &mut usize,
    marker_line: &str,
) -> Result<(), String> {
    let mut pending_text = String::new();
    let mut first_record = true;
    let mut saw_record = false;
    loop {
        let Some((_, line)) = read_note_storage_line(note, reader, offset)? else {
            if !saw_record {
                write_legacy_marker_and_pending_text(write, marker_line, &pending_text)?;
            }
            return Ok(());
        };
        match parse_note_log_line(NoteLogLineKind::Legacy, &line) {
            ParsedNoteLogLine::Skip => {
                if !saw_record {
                    pending_text.push_str(&decode_note_storage_line(&line));
                }
            }
            ParsedNoteLogLine::Record(record) => {
                saw_record = true;
                stream_note_record(note, record, write, first_record)?;
                first_record = false;
            }
            ParsedNoteLogLine::Eof
            | ParsedNoteLogLine::Unfinished
            | ParsedNoteLogLine::Malformed(_)
                if !saw_record =>
            {
                write_legacy_marker_and_pending_text(write, marker_line, &pending_text)?;
                write(&decode_note_storage_line(&line))?;
                stream_decoded_note_text(note, reader, write, offset)?;
                return Ok(());
            }
            ParsedNoteLogLine::Eof | ParsedNoteLogLine::Unfinished => return Ok(()),
            ParsedNoteLogLine::Malformed(err) => {
                return Err(format!(
                    "malformed legacy note log record in {}: {}",
                    note.path.display(),
                    err
                ));
            }
        }
    }
}

fn write_legacy_marker_and_pending_text(
    write: &mut impl FnMut(&str) -> Result<(), String>,
    marker_line: &str,
    pending_text: &str,
) -> Result<(), String> {
    write(&decode_note_storage_line(marker_line))?;
    write(pending_text)
}

fn stream_decoded_note_text(
    note: &Note,
    reader: &mut impl BufRead,
    write: &mut impl FnMut(&str) -> Result<(), String>,
    offset: &mut usize,
) -> Result<(), String> {
    loop {
        let Some((_, line)) = read_note_storage_line(note, reader, offset)? else {
            return Ok(());
        };
        write(&decode_note_storage_line(&line))?;
    }
}

fn read_note_storage_line(
    note: &Note,
    reader: &mut impl BufRead,
    offset: &mut usize,
) -> Result<Option<(usize, String)>, String> {
    let mut line = String::new();
    let line_start = *offset;
    let read = reader
        .read_line(&mut line)
        .map_err(|err| note_read_error(note, err))?;
    if read == 0 {
        return Ok(None);
    }
    *offset += read;
    Ok(Some((line_start, line)))
}

fn find_note_log(note: &Note, content: &str) -> Result<Option<(usize, Vec<NoteRecord>)>, String> {
    for (line_start, line) in lines_with_starts(content) {
        let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
        if is_note_log_marker(note, trimmed, line_start) {
            let log_start = line_start + line.len();
            if let Some(records) = parse_note_log_records(note, &content[log_start..], log_start)? {
                return Ok(Some((line_start.saturating_sub(1), records)));
            }
        } else if trimmed == LEGACY_NOTE_LOG_MARKER {
            // Backward compatibility for note files written before log markers
            // included the note hash and byte offset.
            let log_start = line_start + line.len();
            if let Some(records) = parse_legacy_note_log_records(&content[log_start..]) {
                return Ok(Some((line_start.saturating_sub(1), records)));
            }
        }
    }
    Ok(None)
}

fn parse_note_log_records(
    note: &Note,
    text: &str,
    base_offset: usize,
) -> Result<Option<Vec<NoteRecord>>, String> {
    match collect_note_log_records(
        lines_with_starts(text),
        |relative_start| NoteLogLineKind::Current {
            note,
            line_start: base_offset + relative_start,
        },
        NoteLogCollectPolicy {
            eof: NoteLogLineAction::Skip,
            unfinished: NoteLogLineAction::Stop,
        },
    ) {
        Ok(records) => Ok(Some(records)),
        Err(NoteLogCollectError::Malformed(err)) => Err(format!(
            "malformed note log record in {}: {}",
            note.path.display(),
            err
        )),
        Err(NoteLogCollectError::Eof | NoteLogCollectError::Unfinished) => {
            unreachable!("current note log collection does not fail on EOF or unfinished lines")
        }
    }
}

enum NoteLogLineKind<'a> {
    Current { note: &'a Note, line_start: usize },
    Legacy,
}

enum ParsedNoteLogLine {
    Skip,
    Record(NoteRecord),
    Eof,
    Unfinished,
    Malformed(serde_json::Error),
}

#[derive(Clone, Copy)]
enum NoteLogLineAction {
    Skip,
    Stop,
    Fail,
}

struct NoteLogCollectPolicy {
    eof: NoteLogLineAction,
    unfinished: NoteLogLineAction,
}

enum NoteLogCollectError {
    Eof,
    Unfinished,
    Malformed(serde_json::Error),
}

fn parse_note_log_line(kind: NoteLogLineKind<'_>, line: &str) -> ParsedNoteLogLine {
    let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
    let is_marker = match kind {
        NoteLogLineKind::Current { note, line_start } => {
            is_note_log_marker(note, trimmed, line_start)
        }
        NoteLogLineKind::Legacy => trimmed == LEGACY_NOTE_LOG_MARKER,
    };
    if is_marker || trimmed.trim().is_empty() {
        return ParsedNoteLogLine::Skip;
    }
    match serde_json::from_str(trimmed) {
        Ok(record) => ParsedNoteLogLine::Record(record),
        Err(err) if err.is_eof() => ParsedNoteLogLine::Eof,
        Err(_) if note_log_line_is_unfinished(line) => ParsedNoteLogLine::Unfinished,
        Err(err) => ParsedNoteLogLine::Malformed(err),
    }
}

fn note_log_line_is_unfinished(line: &str) -> bool {
    !line.ends_with('\n')
}

fn parse_legacy_note_log_records(text: &str) -> Option<Vec<NoteRecord>> {
    let records = collect_note_log_records(
        lines_with_starts(text),
        |_| NoteLogLineKind::Legacy,
        NoteLogCollectPolicy {
            eof: NoteLogLineAction::Fail,
            unfinished: NoteLogLineAction::Fail,
        },
    )
    .ok()?;
    (!records.is_empty()).then_some(records)
}

fn collect_note_log_records<'a, 'n>(
    lines: impl Iterator<Item = (usize, &'a str)>,
    mut kind_for_line: impl FnMut(usize) -> NoteLogLineKind<'n>,
    policy: NoteLogCollectPolicy,
) -> Result<Vec<NoteRecord>, NoteLogCollectError> {
    let mut records = Vec::new();
    for (line_start, line) in lines {
        match parse_note_log_line(kind_for_line(line_start), line) {
            ParsedNoteLogLine::Skip => continue,
            ParsedNoteLogLine::Record(record) => records.push(record),
            ParsedNoteLogLine::Eof => match policy.eof {
                NoteLogLineAction::Skip => continue,
                NoteLogLineAction::Stop => break,
                NoteLogLineAction::Fail => return Err(NoteLogCollectError::Eof),
            },
            ParsedNoteLogLine::Unfinished => match policy.unfinished {
                NoteLogLineAction::Skip => continue,
                NoteLogLineAction::Stop => break,
                NoteLogLineAction::Fail => return Err(NoteLogCollectError::Unfinished),
            },
            ParsedNoteLogLine::Malformed(err) => return Err(NoteLogCollectError::Malformed(err)),
        }
    }
    Ok(records)
}

fn lines_with_starts(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut offset = 0;
    text.split_inclusive('\n').map(move |line| {
        let start = offset;
        offset += line.len();
        (start, line)
    })
}

pub(crate) fn read_note_data<T>(
    note: &Note,
    read: impl FnOnce(&Path) -> io::Result<T>,
) -> Result<T, String> {
    reject_symlink(&note.path)?;
    read(&note.path).map_err(|err| note_read_error(note, err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_note_log_records_are_still_readable() {
        let records = parse_legacy_note_log_records(concat!(
            r#"{"op":"append","timestamp":42,"text":"legacy body"}"#,
            "\n"
        ))
        .expect("parse legacy note log");

        assert_eq!(records.len(), 1);
        match &records[0] {
            NoteRecord::Append { timestamp, text } => {
                assert_eq!(*timestamp, 42);
                assert_eq!(text, "legacy body");
            }
            NoteRecord::Write { .. } => panic!("expected append record"),
        }
    }
}

fn note_read_error(note: &Note, err: io::Error) -> String {
    format!("failed to read {}: {}", note.path.display(), err)
}
