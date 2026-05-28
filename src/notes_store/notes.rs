use crate::fs_util::{crossed_size_compaction_bucket, ensure_dir_without_symlinks, reject_symlink};
use crate::hash::hash_key;
#[cfg(any(test, not(unix)))]
use crate::notes_cli::INDEX_LOCK_STALE_AFTER_SECS;
use crate::notes_header::{
    header, initial_content, normalize_body, validate_note_key, verify_note_key,
    verify_note_key_from_first_line,
};
use crate::notes_index::{remove_index, upsert_index, write_file_atomically};
use crate::notes_restore::{
    error_with_restore_context, restore_deleted_note_after_index_failure,
    restore_note_after_index_failure,
};
use crate::output::write_stdout;
use crate::platform::open_file_for_append_without_following_symlink;
use crate::project_types::{Config, Note};
use crate::time::unix_timestamp;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
#[cfg(not(unix))]
use std::path::PathBuf;
#[cfg(any(test, not(unix)))]
use std::time::Duration;

const NOTE_LOG_MARKER: &str = "<!-- canon log v1 -->";
pub(crate) const NOTE_LOG_COMPACT_MIN_BYTES: u64 = 64 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(tag = "op")]
enum NoteRecord {
    #[serde(rename = "write")]
    Write { text: String },
    #[serde(rename = "append")]
    Append { timestamp: u64, text: String },
}

enum NoteTextOperation {
    Write,
    Append,
}

pub(crate) fn ensure_note(config: &Config, key: &str) -> Result<Note, String> {
    let (note, existed, _lock) = locked_note_state(config, key)?;
    if existed {
        return Ok(note);
    } else {
        let content = initial_content(key, &note.hash);
        write_file_atomically(&note.path, content.as_bytes())?;
    }
    upsert_note_index_after_create(config, key, &note)?;
    Ok(note)
}

pub(crate) fn note_for_key(config: &Config, key: &str) -> Result<Note, String> {
    validate_note_key(key)?;
    // A distinct key names retained user data, not a cache entry. Repeated
    // writes/appends to a bounded retained key set are compacted in place; the
    // retained set itself changes only when the user creates or deletes notes.
    let hash = hash_key(key);
    let path = config.root.join(format!("{}.md", hash));
    Ok(Note {
        key: key.to_string(),
        hash,
        path,
    })
}

pub(crate) fn read_note(config: &Config, key: &str) -> Result<(), String> {
    let note = note_for_key(config, key)?;
    if !note.path.exists() {
        return Err(format!("canon not found for key: {}", key));
    }
    let reader = open_note_reader(&note)?;
    stream_note_content(&note, reader, write_stdout)
}

pub(crate) fn write_note(config: &Config, key: &str, text: &str) -> Result<(), String> {
    record_note_text(config, key, text, NoteTextOperation::Write)
}

pub(crate) fn append_note(config: &Config, key: &str, text: &str) -> Result<(), String> {
    record_note_text(config, key, text, NoteTextOperation::Append)
}

fn record_note_text(
    config: &Config,
    key: &str,
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
    append_note_record(config, key, record)
}

fn append_note_record(config: &Config, key: &str, record: NoteRecord) -> Result<(), String> {
    let (note, existed, _lock) = locked_note_state(config, key)?;
    match record {
        NoteRecord::Append { timestamp, text } if existed => {
            let previous_size =
                append_note_log_record(&note.path, &NoteRecord::Append { timestamp, text })?;
            // The append is durable once the log record is flushed and synced.
            // Compaction is opportunistic; reporting its failure would invite
            // retrying an already-persisted append and duplicating visible
            // note content.
            let _ = maybe_compact_note_log(&note, previous_size);
        }
        record => {
            // Replacement writes and first appends both persist the complete
            // note body; existing-note appends take the amortized log path.
            write_compacted_note_record(&note, record)?;
        }
    }
    if !existed {
        upsert_note_index_after_create(config, key, &note)?;
    }
    Ok(())
}

fn upsert_note_index_after_create(config: &Config, key: &str, note: &Note) -> Result<(), String> {
    if let Err(index_err) = upsert_index(config, &note.hash, key) {
        return Err(error_with_restore_context(
            index_err,
            restore_note_after_index_failure(&note.path, None),
        ));
    }
    Ok(())
}

fn append_note_log_record(path: &Path, record: &NoteRecord) -> Result<u64, String> {
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
    entry.push_str(NOTE_LOG_MARKER);
    entry.push('\n');
    entry.push_str(&line);
    if let Err(err) = file
        .write_all(entry.as_bytes())
        .and_then(|()| flush_and_sync_file(&mut file))
    {
        return Err(error_with_restore_context(
            format!("failed to append {}: {}", path.display(), err),
            rollback_note_log_append(path, &mut file, previous_size),
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
    file: &mut fs::File,
    previous_size: u64,
) -> Result<(), String> {
    file.set_len(previous_size)
        .and_then(|()| flush_and_sync_file(file))
        .map_err(|err| {
            format!(
                "failed to roll back {} to {} bytes after append failure: {}",
                path.display(),
                previous_size,
                err
            )
        })
}

#[cfg(test)]
pub(crate) fn rollback_note_log_append_for_test(
    path: &Path,
    previous_size: u64,
) -> Result<(), String> {
    let mut file = open_file_for_append_without_following_symlink(path)?;
    rollback_note_log_append(path, &mut file, previous_size)
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

pub(crate) fn materialize_note_content(note: &Note, content: &str) -> Result<String, String> {
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
    output.push_str(&format!("\n## {}\n\n{}\n", timestamp, body));
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
    line.trim_start_matches('\\') == NOTE_LOG_MARKER
}

fn open_note_reader(note: &Note) -> Result<BufReader<fs::File>, String> {
    reject_symlink(&note.path)?;
    let file = fs::File::open(&note.path)
        .map_err(|err| format!("failed to read {}: {}", note.path.display(), err))?;
    Ok(BufReader::new(file))
}

pub(crate) fn stream_note_content(
    note: &Note,
    mut reader: impl BufRead,
    mut write: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    let mut first_line = String::new();
    if reader
        .read_line(&mut first_line)
        .map_err(|err| format!("failed to read {}: {}", note.path.display(), err))?
        == 0
    {
        return verify_note_key_from_first_line(&note.path, "", &note.key);
    }
    verify_note_key_from_first_line(
        &note.path,
        first_line.trim_end_matches(&['\r', '\n'][..]),
        &note.key,
    )?;
    write(&first_line)?;

    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|err| format!("failed to read {}: {}", note.path.display(), err))?;
        if read == 0 {
            return Ok(());
        }
        let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
        if trimmed == NOTE_LOG_MARKER {
            match stream_note_log(note, &mut reader, &mut write)? {
                NoteLogStream::Applied => return Ok(()),
                NoteLogStream::FalseMarker { first_line } => {
                    write(&decode_note_storage_line(&line))?;
                    if let Some(first_line) = first_line {
                        write(&decode_note_storage_line(&first_line))?;
                    }
                }
            }
            continue;
        }
        write(&decode_note_storage_line(&line))?;
    }
}

enum NoteLogStream {
    Applied,
    FalseMarker { first_line: Option<String> },
}

fn stream_note_log(
    note: &Note,
    reader: &mut impl BufRead,
    write: &mut impl FnMut(&str) -> Result<(), String>,
) -> Result<NoteLogStream, String> {
    let mut first_line = String::new();
    let read = reader
        .read_line(&mut first_line)
        .map_err(|err| format!("failed to read {}: {}", note.path.display(), err))?;
    if read == 0 {
        return Ok(NoteLogStream::FalseMarker { first_line: None });
    }
    match serde_json::from_str::<NoteRecord>(&first_line) {
        Ok(record) => stream_note_record(note, record, write, true)?,
        Err(_) => {
            return Ok(NoteLogStream::FalseMarker {
                first_line: Some(first_line),
            })
        }
    }

    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|err| format!("failed to read {}: {}", note.path.display(), err))?;
        if read == 0 {
            return Ok(NoteLogStream::Applied);
        }
        let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
        if trimmed == NOTE_LOG_MARKER || trimmed.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<NoteRecord>(&line).map_err(|err| {
            format!(
                "malformed note log record in {}: {}",
                note.path.display(),
                err
            )
        })?;
        stream_note_record(note, record, write, false)?;
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
                format!("## {}\n\n{}\n", timestamp, normalize_body(&text))
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

fn find_note_log(note: &Note, content: &str) -> Result<Option<(usize, Vec<NoteRecord>)>, String> {
    let separator = format!("\n{}\n", NOTE_LOG_MARKER);
    let mut offset = 0;
    while let Some(relative_start) = content[offset..].find(&separator) {
        let separator_start = offset + relative_start;
        let log_start = separator_start + separator.len();
        if let Some(records) = parse_note_log_records(note, &content[log_start..])? {
            return Ok(Some((separator_start, records)));
        }
        offset = log_start;
    }
    Ok(None)
}

fn parse_note_log_records(note: &Note, text: &str) -> Result<Option<Vec<NoteRecord>>, String> {
    let mut records = Vec::new();
    for line in text.lines() {
        if line == NOTE_LOG_MARKER || line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(record) => records.push(record),
            Err(_) if records.is_empty() => return Ok(None),
            Err(err) => {
                return Err(format!(
                    "malformed note log record in {}: {}",
                    note.path.display(),
                    err
                ));
            }
        }
    }
    Ok((!records.is_empty()).then_some(records))
}

pub(crate) fn delete_note(config: &Config, key: &str) -> Result<(), String> {
    let (note, existed, _lock) = locked_note_state(config, key)?;
    if existed {
        let original = read_note_data(&note, |path| fs::read(path))?;
        fs::remove_file(&note.path)
            .map_err(|err| format!("failed to delete {}: {}", note.path.display(), err))?;
        if let Err(index_err) = remove_index(config, &note.hash, key) {
            return Err(error_with_restore_context(
                index_err,
                restore_deleted_note_after_index_failure(&note.path, &original),
            ));
        }
    } else {
        remove_index(config, &note.hash, key)?;
    }
    Ok(())
}

// Note compaction replaces the note file, so same-note mutations use a sidecar
// lock that stays stable across append-log writes, compaction, and delete.
#[cfg(unix)]
struct NoteLock {
    _file: fs::File,
}

#[cfg(not(unix))]
struct NoteLock {
    _file: fs::File,
    path: PathBuf,
}

#[cfg(not(unix))]
impl Drop for NoteLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn lock_note(note: &Note) -> Result<NoteLock, String> {
    let path = note.path.with_extension("md.lock");
    lock_note_at_path(&path)
}

#[cfg(unix)]
fn lock_note_at_path(path: &Path) -> Result<NoteLock, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW);
    let file = options
        .open(&path)
        .map_err(|err| format!("failed to open lock {}: {}", path.display(), err))?;
    lock_note_file(&file, &path)?;
    Ok(NoteLock { _file: file })
}

#[cfg(not(unix))]
fn lock_note_at_path(path: &Path) -> Result<NoteLock, String> {
    match create_note_lock(path) {
        Ok(file) => Ok(NoteLock {
            _file: file,
            path: path.to_path_buf(),
        }),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            if !note_lock_is_stale(path)? {
                return Err(format!(
                    "failed to lock {}: lock is already held",
                    path.display()
                ));
            }
            remove_stale_note_lock(path)?;
            let file = create_note_lock(path)
                .map_err(|err| format!("failed to lock {}: {}", path.display(), err))?;
            Ok(NoteLock {
                _file: file,
                path: path.to_path_buf(),
            })
        }
        Err(err) => Err(format!("failed to lock {}: {}", path.display(), err)),
    }
}

#[cfg(not(unix))]
fn create_note_lock(path: &Path) -> Result<fs::File, io::Error> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(any(test, not(unix)))]
pub(crate) fn stale_note_lock_age(age: Duration) -> bool {
    age >= Duration::from_secs(INDEX_LOCK_STALE_AFTER_SECS)
}

#[cfg(not(unix))]
fn note_lock_is_stale(path: &Path) -> Result<bool, String> {
    reject_symlink(path)?;
    let metadata = fs::metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
    let modified = metadata
        .modified()
        .map_err(|err| format!("failed to inspect mtime for {}: {}", path.display(), err))?;
    let age = modified
        .elapsed()
        .map_err(|err| format!("failed to inspect age for {}: {}", path.display(), err))?;
    Ok(stale_note_lock_age(age))
}

#[cfg(not(unix))]
fn remove_stale_note_lock(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!(
            "failed to remove stale lock {}: {}",
            path.display(),
            err
        )),
    }
}

#[cfg(unix)]
fn lock_note_file(file: &fs::File, path: &Path) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    loop {
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if result == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return Err(format!("failed to lock {}: {}", path.display(), err));
    }
}

fn locked_note_state(config: &Config, key: &str) -> Result<(Note, bool, NoteLock), String> {
    ensure_dir_without_symlinks(&config.root)?;
    let note = note_for_key(config, key)?;
    let lock = lock_note(&note)?;
    let existed = note.path.exists();
    if existed {
        verify_note_key(&note.path, key)?;
    }
    Ok((note, existed, lock))
}

fn read_note_data<T>(note: &Note, read: impl FnOnce(&Path) -> io::Result<T>) -> Result<T, String> {
    reject_symlink(&note.path)?;
    read(&note.path).map_err(|err| format!("failed to read {}: {}", note.path.display(), err))
}
