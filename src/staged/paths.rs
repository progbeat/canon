use std::io;
use std::path::{Path, PathBuf};
use std::process;

const CANON_TREE_CACHE_DIR: &str = "CANON_TREE_CACHE_DIR";

// Hardlink materialization cannot be evaluated from this file alone: paths.rs
// owns the temporary-directory selection, and worktree/mod.rs owns
// lazy_tree_dir, trees_dir, unpacked_paths, visible_tree.entry_paths, and
// materialize().
pub(crate) struct TemporaryMaterializationRoot {
    tmp_dir: PathBuf,
    ownership: TemporaryDirectoryOwnership,
}

enum TemporaryDirectoryOwnership {
    Caller,
    Canon,
}

impl TemporaryMaterializationRoot {
    pub(crate) fn tmp_dir(&self) -> &Path {
        &self.tmp_dir
    }

    pub(crate) fn is_canon_owned(&self) -> bool {
        matches!(self.ownership, TemporaryDirectoryOwnership::Canon)
    }
}

pub(crate) fn create_temporary_materialization_root(
    _root: &Path,
) -> Result<TemporaryMaterializationRoot, String> {
    if let Some(tmp_dir) = configured_tmp_dir() {
        return caller_owned_temporary_root(&tmp_dir);
    }
    make_temp_dir().map(canon_owned_temporary_root)
}

fn make_temp_dir() -> Result<PathBuf, String> {
    let mut errors = Vec::new();
    // This is the hardlink materialization policy's `make_temp_dir()`.
    // Canon's temporary-storage expectation defines the parent preference.
    if let Some(path) = create_tmp_dir_from_candidates(
        crate::platform::memory_backed_staged_snapshot_parent_candidates(),
        &mut errors,
    ) {
        return Ok(path);
    }
    if let Some(path) = create_tmp_dir_from_candidates(
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

fn configured_tmp_dir() -> Option<PathBuf> {
    let value = std::env::var_os(CANON_TREE_CACHE_DIR)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

fn caller_owned_temporary_root(tmp_dir: &Path) -> Result<TemporaryMaterializationRoot, String> {
    // CANON_TREE_CACHE_DIR supplies the policy's caller-owned tmp_dir. It is
    // temporary storage, not project-owned persistent state.
    crate::platform::create_private_dir_all(tmp_dir).map_err(|err| {
        format!(
            "failed to create {} {}: {}",
            CANON_TREE_CACHE_DIR,
            tmp_dir.display(),
            err
        )
    })?;
    Ok(TemporaryMaterializationRoot {
        tmp_dir: tmp_dir.to_path_buf(),
        ownership: TemporaryDirectoryOwnership::Caller,
    })
}

fn canon_owned_temporary_root(tmp_dir: PathBuf) -> TemporaryMaterializationRoot {
    TemporaryMaterializationRoot {
        tmp_dir,
        ownership: TemporaryDirectoryOwnership::Canon,
    }
}

fn create_tmp_dir_from_candidates(
    parents: Vec<PathBuf>,
    errors: &mut Vec<String>,
) -> Option<PathBuf> {
    for parent in parents {
        let path = match create_tmp_dir_in(&parent) {
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

fn create_tmp_dir_in(parent: &Path) -> Result<PathBuf, String> {
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
