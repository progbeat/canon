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

pub(crate) fn create_snapshot_root(_root: &Path) -> Result<SnapshotRoot, String> {
    if let Some(path) = configured_tree_cache_dir() {
        return create_snapshot_root_from_configured_cache_dir(&path);
    }
    let mut errors = Vec::new();
    // This is Canon's `make_temp_dir()` for hardlink materialization when
    // CANON_TREE_CACHE_DIR is unset. A separate canon expectation constrains
    // canon-owned temporary storage to prefer memory-backed parents when the
    // host provides one; ordinary temp storage remains the fallback.
    if let Some(path) = create_snapshot_root_from_candidates(
        crate::platform::memory_backed_staged_snapshot_parent_candidates(),
        &mut errors,
    ) {
        return Ok(temporary_snapshot_root(path));
    }
    if let Some(path) = create_snapshot_root_from_candidates(
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

fn create_snapshot_root_from_configured_cache_dir(path: &Path) -> Result<SnapshotRoot, String> {
    crate::platform::create_private_dir_all(path).map_err(|err| {
        format!(
            "failed to create {} {}: {}",
            CANON_TREE_CACHE_DIR,
            path.display(),
            err
        )
    })?;
    let path = path.canonicalize().map_err(|err| {
        format!(
            "failed to canonicalize {} {}: {}",
            CANON_TREE_CACHE_DIR,
            path.display(),
            err
        )
    })?;
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
