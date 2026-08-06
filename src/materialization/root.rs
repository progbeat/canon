use super::root_lock::MaterializationRootLock;
use crate::platform::filesystem::{
    OwnedPrivateTemporaryDirectory, PrivateTemporaryDirectoryAllocator,
};
use std::path::{Path, PathBuf};

const CANON_TREE_CACHE_DIR: &str = "CANON_TREE_CACHE_DIR";
const MATERIALIZATION_ROOT_PREFIX: &str = "canon-tree-materialization";

// Hardlink materialization cannot be evaluated from this file alone: this
// module owns storage-root selection, while the sibling modules own the
// filesystem-shaped checked project input and its materialization.
pub(crate) struct MaterializedProjectInputRoot {
    tmp_dir: PathBuf,
    owner: TemporaryDirectoryOwner,
    restores_caller_root_on_drop: bool,
}

enum TemporaryDirectoryOwner {
    Caller {
        _lock: MaterializationRootLock,
    },
    Canon {
        _owner: OwnedPrivateTemporaryDirectory,
    },
}

impl MaterializedProjectInputRoot {
    fn new(
        tmp_dir: PathBuf,
        owner: TemporaryDirectoryOwner,
        restores_caller_root_on_drop: bool,
    ) -> Self {
        Self {
            tmp_dir,
            owner,
            restores_caller_root_on_drop,
        }
    }

    pub(crate) fn tmp_dir(&self) -> &Path {
        &self.tmp_dir
    }

    pub(crate) fn is_canon_owned(&self) -> bool {
        matches!(self.owner, TemporaryDirectoryOwner::Canon { .. })
    }

    pub(crate) fn restores_caller_root_on_drop(&self) -> bool {
        self.restores_caller_root_on_drop
    }
}

pub(crate) fn create_project_input_root(
    temporary_directory_allocator: &PrivateTemporaryDirectoryAllocator,
) -> Result<MaterializedProjectInputRoot, String> {
    if let Some(tmp_dir) = configured_tmp_dir() {
        return create_configured_project_input_root(tmp_dir, false);
    }
    create_canon_owned_project_input_root(temporary_directory_allocator)
}

pub(crate) fn create_rollback_project_input_root(
    temporary_directory_allocator: &PrivateTemporaryDirectoryAllocator,
) -> Result<MaterializedProjectInputRoot, String> {
    create_rollback_project_input_root_for(configured_tmp_dir(), temporary_directory_allocator)
}

fn create_rollback_project_input_root_for(
    configured_tmp_dir: Option<PathBuf>,
    temporary_directory_allocator: &PrivateTemporaryDirectoryAllocator,
) -> Result<MaterializedProjectInputRoot, String> {
    // [1t,l] CANON_TREE_CACHE_DIR is the hardlink policy's exact tmp_dir.
    // A configured root is a caller-selected shared namespace even when this
    // invocation creates the directory. The lifetime lock prevents another
    // invocation from using entries that rollback may remove; rollback never
    // removes the shared root itself.
    if let Some(tmp_dir) = configured_tmp_dir {
        return create_configured_project_input_root(tmp_dir, true);
    }
    create_canon_owned_project_input_root(temporary_directory_allocator)
}

fn configured_tmp_dir() -> Option<PathBuf> {
    let value = std::env::var_os(CANON_TREE_CACHE_DIR)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

fn create_configured_project_input_root(
    tmp_dir: PathBuf,
    restores_caller_root_on_drop: bool,
) -> Result<MaterializedProjectInputRoot, String> {
    // CANON_TREE_CACHE_DIR supplies the policy's exact tmp_dir. It is temporary
    // storage, not project-owned persistent state.
    create_configured_materialization_dir(&tmp_dir)?;
    let owner = TemporaryDirectoryOwner::Caller {
        _lock: MaterializationRootLock::acquire(&tmp_dir)?,
    };
    Ok(MaterializedProjectInputRoot::new(
        tmp_dir,
        owner,
        restores_caller_root_on_drop,
    ))
}

fn create_configured_materialization_dir(tmp_dir: &Path) -> Result<(), String> {
    crate::platform::filesystem::create_private_dir_all(tmp_dir).map_err(|err| {
        format!(
            "failed to create {} {}: {}",
            CANON_TREE_CACHE_DIR,
            tmp_dir.display(),
            err
        )
    })
}

fn create_canon_owned_project_input_root(
    temporary_directory_allocator: &PrivateTemporaryDirectoryAllocator,
) -> Result<MaterializedProjectInputRoot, String> {
    let owner = OwnedPrivateTemporaryDirectory::create(
        temporary_directory_allocator,
        MATERIALIZATION_ROOT_PREFIX,
    )?;
    Ok(MaterializedProjectInputRoot::new(
        owner.path().to_path_buf(),
        TemporaryDirectoryOwner::Canon { _owner: owner },
        false,
    ))
}
