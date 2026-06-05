use crate::fs_util::ensure_dir_without_symlinks;
use crate::platform;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

pub(super) fn path_exists_no_follow(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(false)
        }
        Err(err) => Err(format!("failed to inspect {}: {}", path.display(), err)),
    }
}

pub(super) fn ensure_project_dir_without_symlinks(root: &Path, path: &Path) -> Result<(), String> {
    path.strip_prefix(root).map_err(|_| {
        format!(
            "refusing to create directory outside project root: {}",
            path.display()
        )
    })?;
    ensure_dir_without_symlinks(path)
}

pub(super) fn write_new_file(path: &Path, content: &str) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| format!("failed to create {}: {}", path.display(), err))?;
    file.write_all(content.as_bytes())
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    file.flush()
        .map_err(|err| format!("failed to flush {}: {}", path.display(), err))
}

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
