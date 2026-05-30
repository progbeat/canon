use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;

const CANON_TREE_CACHE_DIR: &str = "CANON_TREE_CACHE_DIR";

pub(crate) struct SnapshotRoot {
    path: PathBuf,
    remove_on_drop: bool,
}

impl SnapshotRoot {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn remove_on_drop(&self) -> bool {
        self.remove_on_drop
    }
}

pub(crate) fn create_snapshot_root(root: &Path) -> Result<SnapshotRoot, String> {
    let root = root.canonicalize().map_err(|err| {
        format!(
            "failed to canonicalize project root {}: {}",
            root.display(),
            err
        )
    })?;
    if let Some(path) = configured_tree_cache_dir() {
        return create_snapshot_root_from_configured_cache_dir(&root, &path);
    }
    let mut errors = Vec::new();
    // The lazy hardlink policy prefers memory-backed temporary storage when
    // the host provides a usable parent. Platform discovery is centralized in
    // `platform::memory_backed_staged_snapshot_parent_candidates`: Unix adds
    // common RAM paths and /proc-discovered tmpfs/ramfs mounts, while other
    // platforms can expose explicit RAM-disk temp roots. Ordinary temporary
    // storage is tried only after every discovered memory-backed parent is
    // missing, inside the worktree, or unusable.
    if let Some(path) = create_snapshot_root_from_candidates(
        &root,
        crate::platform::memory_backed_staged_snapshot_parent_candidates(),
        &mut errors,
    ) {
        return Ok(temporary_snapshot_root(path));
    }
    if let Some(path) = create_snapshot_root_from_candidates(
        &root,
        crate::platform::ordinary_staged_snapshot_parent_candidates(),
        &mut errors,
    ) {
        return Ok(temporary_snapshot_root(path));
    }
    Err(format!(
        "failed to create staged snapshot directory: {}",
        errors.join("; ")
    ))
}

fn configured_tree_cache_dir() -> Option<PathBuf> {
    let value = std::env::var_os(CANON_TREE_CACHE_DIR)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

fn create_snapshot_root_from_configured_cache_dir(
    root: &Path,
    path: &Path,
) -> Result<SnapshotRoot, String> {
    crate::platform::create_private_dir_all(path).map_err(|err| {
        format!(
            "failed to create {} {}: {}",
            CANON_TREE_CACHE_DIR,
            path.display(),
            err
        )
    })?;
    let path = canonical_snapshot_path_outside_worktree(root, path, CANON_TREE_CACHE_DIR, None)?;
    Ok(SnapshotRoot {
        path,
        remove_on_drop: false,
    })
}

fn temporary_snapshot_root(path: PathBuf) -> SnapshotRoot {
    SnapshotRoot {
        path,
        remove_on_drop: true,
    }
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
