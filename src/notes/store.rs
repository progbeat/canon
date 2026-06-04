use crate::fs_util::ensure_dir_without_symlinks;
use crate::hash::hash_key;
use crate::notes::header::{initial_content, validate_note_key, verify_note_key};
use crate::notes::index::{remove_index, upsert_index, write_file_atomically};
use crate::notes::note_lock::{lock_note, NoteLock};
use crate::notes::note_log::{self, NoteTextOperation};
use crate::notes::restore::{
    error_with_restore_context, restore_deleted_note_after_index_failure,
    restore_note_after_index_failure,
};
use crate::output::write_stdout;
use crate::project_types::{Config, Note};
use std::fs;

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
    note_log::read_note_content(&note, write_stdout)
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
    let (note, existed, _lock) = locked_note_state(config, key)?;
    note_log::record_note_text(&note, existed, text, operation)?;
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

pub(crate) fn delete_note(config: &Config, key: &str) -> Result<(), String> {
    let (note, existed, _lock) = locked_note_state(config, key)?;
    if existed {
        let original = note_log::read_note_data(&note, |path| fs::read(path))?;
        fs::remove_file(&note.path)
            .map_err(|err| format!("failed to delete {}: {}", note.path.display(), err))?;
        if let Err(index_err) = remove_index(config, key) {
            return Err(error_with_restore_context(
                index_err,
                restore_deleted_note_after_index_failure(&note.path, &original),
            ));
        }
    } else {
        remove_index(config, key)?;
    }
    Ok(())
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
