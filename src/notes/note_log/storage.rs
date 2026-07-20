use crate::project_types::Note;

pub(super) const LEGACY_NOTE_LOG_MARKER: &str = "<!-- canon log v1 -->";
const NOTE_LOG_MARKER_PREFIX: &str = "<!-- canon log v1 ";
const NOTE_LOG_MARKER_SUFFIX: &str = " -->";

pub(super) fn encode_note_body_for_storage(text: &str) -> String {
    crate::notes::header::normalize_body(text)
        .split('\n')
        .map(encode_note_storage_line)
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn decode_note_storage_line(line: &str) -> String {
    let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
    let suffix = &line[trimmed.len()..];
    if trimmed.strip_prefix('\\').is_some_and(is_log_marker_family) {
        format!("{}{}", &trimmed[1..], suffix)
    } else {
        line.to_string()
    }
}

pub(super) fn note_log_marker(note: &Note, marker_offset: u64) -> String {
    format!(
        "{}hash={} offset={}{}",
        NOTE_LOG_MARKER_PREFIX, note.hash, marker_offset, NOTE_LOG_MARKER_SUFFIX
    )
}

pub(super) fn is_note_log_marker(note: &Note, line: &str, line_start: usize) -> bool {
    let Some((hash, offset)) = parse_note_log_marker(line) else {
        return false;
    };
    hash == note.hash && offset == line_start as u64
}

fn encode_note_storage_line(line: &str) -> String {
    if is_log_marker_family(line) {
        format!("\\{}", line)
    } else {
        line.to_string()
    }
}

fn is_log_marker_family(line: &str) -> bool {
    let line = line.trim_start_matches('\\');
    line == LEGACY_NOTE_LOG_MARKER
        || (line.starts_with(NOTE_LOG_MARKER_PREFIX) && line.ends_with(NOTE_LOG_MARKER_SUFFIX))
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
        } else {
            let value = part.strip_prefix("offset=")?;
            offset = value.parse::<u64>().ok();
        }
    }
    Some((hash?, offset?))
}
