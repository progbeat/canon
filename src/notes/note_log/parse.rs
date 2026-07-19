use super::record::NoteRecord;
use super::storage::{is_note_log_marker, LEGACY_NOTE_LOG_MARKER};
use crate::project_types::Note;

pub(super) fn find_note_log(
    note: &Note,
    content: &str,
) -> Result<Option<(usize, Vec<NoteRecord>)>, String> {
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

pub(super) enum NoteLogLineKind<'a> {
    Current { note: &'a Note, line_start: usize },
    Legacy,
}

pub(super) enum ParsedNoteLogLine {
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

pub(super) fn parse_note_log_line(kind: NoteLogLineKind<'_>, line: &str) -> ParsedNoteLogLine {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: Sx
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
