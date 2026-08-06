use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;

const DELETE_ACCESS: u32 = 0x0001_0000;
const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const LOCK_FILE_NAME: &str = ".canon-tree-materialization.lock";

pub(super) struct MaterializationRootLock {
    _file: File,
}

impl MaterializationRootLock {
    pub(super) fn acquire(root: &Path) -> Result<Self, String> {
        let path = root.join(LOCK_FILE_NAME);
        let file = open_lock_file(&path)?;
        file.lock().map_err(|err| {
            format!(
                "failed to lock caller-owned materialization root {}: {}",
                root.display(),
                err
            )
        })?;
        Ok(Self { _file: file })
    }
}

fn open_lock_file(path: &Path) -> Result<File, String> {
    loop {
        match create_ephemeral_lock_file(path) {
            Ok(file) => return Ok(file),
            Err(create_error) if create_error.kind() == ErrorKind::AlreadyExists => {
                match OpenOptions::new().read(true).write(true).open(path) {
                    Ok(file) => return Ok(file),
                    Err(open_error) if open_error.kind() == ErrorKind::NotFound => continue,
                    Err(open_error) => {
                        return Err(open_error_message(path, create_error, open_error))
                    }
                }
            }
            Err(create_error) => {
                return Err(format!(
                    "failed to create materialization lock file {}: {}",
                    path.display(),
                    create_error
                ));
            }
        }
    }
}

fn create_ephemeral_lock_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE_ACCESS)
        .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
        .open(path)
}

fn open_error_message(
    path: &Path,
    create_error: std::io::Error,
    open_error: std::io::Error,
) -> String {
    format!(
        "failed to create or open materialization lock file {}: create: {}; open: {}",
        path.display(),
        create_error,
        open_error
    )
}
