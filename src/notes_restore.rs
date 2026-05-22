use crate::notes_index::write_file_atomically;
use std::fs;
use std::io;
use std::path::Path;

pub(crate) fn restore_note_after_index_failure(
    path: &Path,
    original: Option<&[u8]>,
) -> Result<(), String> {
    match original {
        Some(content) => restore_deleted_note_after_index_failure(path, content),
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!(
                "failed to remove {} after index update failure: {}",
                path.display(),
                err
            )),
        },
    }
}

pub(crate) fn restore_deleted_note_after_index_failure(
    path: &Path,
    original: &[u8],
) -> Result<(), String> {
    write_file_atomically(path, original).map_err(|err| {
        format!(
            "failed to restore {} after index update failure: {}",
            path.display(),
            err
        )
    })
}

pub(crate) fn error_with_restore_context(
    index_error: String,
    restore_result: Result<(), String>,
) -> String {
    match restore_result {
        Ok(()) => index_error,
        Err(restore_error) => format!("{}; additionally {}", index_error, restore_error),
    }
}
