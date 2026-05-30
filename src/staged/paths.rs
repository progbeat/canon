use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;

pub(crate) fn create_snapshot_root(root: &Path) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|err| {
        format!(
            "failed to canonicalize project root {}: {}",
            root.display(),
            err
        )
    })?;
    let mut errors = Vec::new();
    // The lazy hardlink policy prefers memory-backed temporary storage when
    // the host provides a usable parent. Ordinary temporary storage is tried
    // only after every discovered memory-backed parent is missing, inside the
    // worktree, or unusable.
    if let Some(path) = create_snapshot_root_from_candidates(
        &root,
        crate::platform::memory_backed_staged_snapshot_parent_candidates(),
        &mut errors,
    ) {
        return Ok(path);
    }
    if let Some(path) = create_snapshot_root_from_candidates(
        &root,
        crate::platform::ordinary_staged_snapshot_parent_candidates(),
        &mut errors,
    ) {
        return Ok(path);
    }
    Err(format!(
        "failed to create staged snapshot directory: {}",
        errors.join("; ")
    ))
}

fn create_snapshot_root_from_candidates(
    root: &Path,
    parents: Vec<PathBuf>,
    errors: &mut Vec<String>,
) -> Option<PathBuf> {
    for parent in parents {
        if let Err(err) = snapshot_parent_outside_worktree(root, &parent) {
            errors.push(err);
            continue;
        }
        let path = match create_snapshot_root_in(&parent) {
            Ok(path) => path,
            Err(err) => {
                errors.push(err);
                continue;
            }
        };
        match verify_snapshot_root_outside_worktree(root, path) {
            Ok(path) => return Some(path),
            Err(err) => errors.push(err),
        }
    }
    None
}

pub(crate) fn snapshot_parent_outside_worktree(root: &Path, parent: &Path) -> Result<(), String> {
    canonical_snapshot_path_outside_worktree(root, parent, "staged snapshot parent", None)
        .map(|_| ())
}

fn verify_snapshot_root_outside_worktree(root: &Path, path: PathBuf) -> Result<PathBuf, String> {
    canonical_snapshot_path_outside_worktree(
        root,
        &path,
        "staged snapshot root",
        Some(path.as_path()),
    )?;
    Ok(path)
}

fn canonical_snapshot_path_outside_worktree(
    root: &Path,
    path: &Path,
    description: &str,
    cleanup_on_inside: Option<&Path>,
) -> Result<PathBuf, String> {
    let canonical = path.canonicalize().map_err(|err| {
        format!(
            "failed to canonicalize {} {}: {}",
            description,
            path.display(),
            err
        )
    })?;
    if canonical == root || canonical.starts_with(root) {
        if let Some(cleanup) = cleanup_on_inside {
            let _ = fs::remove_dir_all(cleanup);
        }
        return Err(format!(
            "{} {} is inside project root {}",
            description,
            canonical.display(),
            root.display()
        ));
    }
    Ok(canonical)
}

fn create_snapshot_root_in(parent: &Path) -> Result<PathBuf, String> {
    if !parent.is_dir() {
        return Err(format!("{} is not a directory", parent.display()));
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    for attempt in 0..1000 {
        let path = parent.join(format!(
            "canon-check-snapshot-{}-{}-{}",
            process::id(),
            stamp,
            attempt
        ));
        match crate::platform::create_private_dir(&path) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(format!("failed to create {}: {}", path.display(), err));
            }
        }
    }
    Err(format!(
        "failed to allocate a unique staged snapshot directory under {}",
        parent.display()
    ))
}
