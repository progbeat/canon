use crate::platform::filesystem;
use std::fs;
use std::path::Path;

pub(super) enum HookFile {
    Missing,
    Regular(String),
    Unverifiable,
}

pub(super) fn make_executable(path: &Path) -> Result<(), String> {
    filesystem::make_hook_executable(path)
}

pub(super) fn inspect_hook_file(path: &Path) -> HookFile {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return HookFile::Missing,
        Err(_) => return HookFile::Unverifiable,
    };
    if !metadata.file_type().is_file() {
        return HookFile::Unverifiable;
    }
    match fs::read_to_string(path) {
        Ok(contents) => HookFile::Regular(contents),
        Err(_) => HookFile::Unverifiable,
    }
}
