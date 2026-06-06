use super::materialize::{maybe_compact_note_log, write_compacted_note_record};
use super::record::{NoteRecord, NoteTextOperation};
use super::storage::note_log_marker;
use crate::notes::header::normalize_body;
use crate::notes::restore::error_with_restore_context;
use crate::project_types::Note;
use crate::time::unix_timestamp;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

#[cfg(unix)]
#[path = "append/platform_unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "append/platform_windows.rs"]
mod platform;

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
    let mut file = platform::open_append_target(path)?;
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
    if platform::rollback_needs_reopen(&err) {
        drop(file);
        let mut file = platform::open_rollback_target(path)?;
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

#[cfg(test)]
pub(crate) fn rollback_note_log_append_for_test(
    path: &Path,
    previous_size: u64,
) -> Result<(), String> {
    let file = platform::open_append_target(path)?;
    rollback_note_log_append(path, file, previous_size)
}
