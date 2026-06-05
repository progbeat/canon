use super::file::{open_note_reader, read_note_storage_line};
use super::materialize::{append_visible_note_section, note_section};
use super::parse::{parse_note_log_line, NoteLogLineKind, ParsedNoteLogLine};
use super::record::NoteRecord;
use super::storage::{decode_note_storage_line, is_note_log_marker, LEGACY_NOTE_LOG_MARKER};
use crate::notes::header::{normalize_body, verify_note_key_from_first_line};
use crate::project_types::Note;
use std::io::BufRead;

pub(crate) fn read_note_content(
    note: &Note,
    write: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    let reader = open_note_reader(note)?;
    stream_note_content(note, reader, write)
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
