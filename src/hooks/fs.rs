use crate::platform;
use std::fs;
use std::io;
use std::path::Path;

pub(super) fn make_executable(path: &Path) -> Result<(), String> {
    platform::make_hook_executable(path)
}

pub(super) fn read_optional_file(path: &Path) -> Result<Option<String>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(err) => return Err(format!("failed to read {}: {}", path.display(), err)),
    };
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "refusing to use symlinked pre-commit hook {}",
            path.display()
        ));
    }
    if !metadata.file_type().is_file() {
        return Err(format!(
            "refusing to use non-file pre-commit hook {}",
            path.display()
        ));
    }
    fs::read_to_string(path)
        .map(Some)
        .map_err(|err| format!("failed to read regular file {}: {}", path.display(), err))
}
