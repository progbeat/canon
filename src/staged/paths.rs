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
    InvocationLocalCaller,
}

impl TemporaryMaterializationRoot {
    pub(crate) fn tmp_dir(&self) -> &Path {
        &self.tmp_dir
    }

    pub(crate) fn is_canon_owned(&self) -> bool {
        matches!(self.ownership, TemporaryDirectoryOwnership::Canon)
    }

    pub(crate) fn restores_invocation_local_artifacts(&self) -> bool {
        matches!(
            self.ownership,
            TemporaryDirectoryOwnership::InvocationLocalCaller
        )
    }
}

pub(crate) fn create_temporary_materialization_root() -> Result<TemporaryMaterializationRoot, String>
{
    if let Some(tmp_dir) = configured_tmp_dir() {
        return caller_owned_temporary_root(&tmp_dir);
    }
    make_temp_dir().map(canon_owned_temporary_root)
}

pub(crate) fn create_invocation_local_materialization_root(
) -> Result<TemporaryMaterializationRoot, String> {
    create_invocation_local_materialization_root_for(configured_tmp_dir())
}

pub(super) fn create_invocation_local_materialization_root_for(
    configured_parent: Option<PathBuf>,
) -> Result<TemporaryMaterializationRoot, String> {
    // [ig,Ky] CANON_TREE_CACHE_DIR is the hardlink policy's exact tmp_dir.
    // When it already exists, the staged view journals and restores only its
    // own changes. When Canon creates it for this invocation, normal owned-root
    // cleanup removes the whole directory.
    if let Some(tmp_dir) = configured_parent {
        let existed = tmp_dir.is_dir();
        crate::platform::create_private_dir_all(&tmp_dir).map_err(|err| {
            format!(
                "failed to create {} {}: {}",
                CANON_TREE_CACHE_DIR,
                tmp_dir.display(),
                err
            )
        })?;
        return Ok(TemporaryMaterializationRoot {
            tmp_dir,
            ownership: if existed {
                TemporaryDirectoryOwnership::InvocationLocalCaller
            } else {
                TemporaryDirectoryOwnership::Canon
            },
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: ig,Ky
    fn invocation_local_root_is_the_configured_tree_cache_dir_with_restore_ownership() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let configured_parent = std::env::temp_dir().join(format!(
            "canon-invocation-local-tree-cache-parent-{}-{unique}",
            process::id()
        ));
        fs::create_dir_all(&configured_parent).unwrap();
        let root =
            create_invocation_local_materialization_root_for(Some(configured_parent.clone()))
                .unwrap();

        assert!(!root.is_canon_owned());
        assert!(root.restores_invocation_local_artifacts());
        assert_eq!(root.tmp_dir(), configured_parent);

        fs::remove_dir_all(configured_parent).unwrap();
    }
}
