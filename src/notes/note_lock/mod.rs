use crate::project_types::Note;
use std::time::Duration;

const NOTE_LOCK_STALE_AFTER_SECS: u64 = 600;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

pub(crate) struct NoteLock {
    _inner: imp::NoteLock,
}

// Note compaction replaces the note file, so same-note mutations use a sidecar
// lock that stays stable across append-log writes, compaction, and delete.
pub(crate) fn lock_note(note: &Note) -> Result<NoteLock, String> {
    let path = note.path.with_extension("md.lock");
    imp::lock_note_sidecar(&path, Duration::from_secs(NOTE_LOCK_STALE_AFTER_SECS))
        .map(|inner| NoteLock { _inner: inner })
}
