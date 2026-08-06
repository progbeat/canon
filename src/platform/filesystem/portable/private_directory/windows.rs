use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(super) fn create_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir(path)?;
    set_private_permissions_io(path)
}

pub(super) fn create_private_dir_all(path: &Path) -> io::Result<()> {
    let missing = missing_ancestors(path);
    fs::create_dir_all(path)?;
    if missing.is_empty() {
        return set_private_permissions_io(path);
    }
    for dir in missing.into_iter().rev() {
        set_private_permissions_io(&dir)?;
    }
    Ok(())
}

fn missing_ancestors(path: &Path) -> Vec<PathBuf> {
    let mut missing = Vec::new();
    let mut current = path;
    while !current.exists() {
        missing.push(current.to_path_buf());
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }
    missing
}

fn set_private_permissions_io(path: &Path) -> io::Result<()> {
    super::super::permissions::windows::set_private_permissions(path).map_err(io::Error::other)
}
