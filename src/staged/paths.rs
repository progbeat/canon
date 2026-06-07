use std::io;
use std::path::{Path, PathBuf};
use std::process;

const CANON_TREE_CACHE_DIR: &str = "CANON_TREE_CACHE_DIR";

// Hardlink materialization cannot be evaluated from this file alone:
// paths.rs owns tmp_dir selection, and worktree/mod.rs owns lazy_tree_dir,
// trees_dir, unpacked_paths, visible_tree.entry_paths, and materialize().
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

pub(crate) fn create_snapshot_root(_root: &Path) -> Result<SnapshotRoot, String> {
    if let Some(path) = configured_tree_cache_dir() {
        return create_snapshot_root_from_configured_cache_dir(&path);
    }
    make_temp_dir().map(temporary_snapshot_root)
}

fn make_temp_dir() -> Result<PathBuf, String> {
    let mut errors = Vec::new();
    // This is the hardlink materialization policy's `make_temp_dir()`.
    // Canon's temporary-storage expectation defines the parent preference.
    if let Some(path) = create_snapshot_root_from_candidates(
        crate::platform::memory_backed_staged_snapshot_parent_candidates(),
        &mut errors,
    ) {
        return Ok(path);
    }
    if let Some(path) = create_snapshot_root_from_candidates(
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

fn configured_tree_cache_dir() -> Option<PathBuf> {
    let value = std::env::var_os(CANON_TREE_CACHE_DIR)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

fn create_snapshot_root_from_configured_cache_dir(path: &Path) -> Result<SnapshotRoot, String> {
    // This file only owns the hardlink policy's tmp_dir selection. The
    // remaining fields and materialize() flow live in src/staged/worktree/mod.rs.
    crate::platform::create_private_dir_all(path).map_err(|err| {
        format!(
            "failed to create {} {}: {}",
            CANON_TREE_CACHE_DIR,
            path.display(),
            err
        )
    })?;
    Ok(SnapshotRoot {
        path: path.to_path_buf(),
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
    parents: Vec<PathBuf>,
    errors: &mut Vec<String>,
) -> Option<PathBuf> {
    for parent in parents {
        let path = match create_snapshot_root_in(&parent) {
            Ok(path) => path,
            Err(err) => {
                errors.push(err);
                continue;
            }
        };
        return Some(path);
    }
    None
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
