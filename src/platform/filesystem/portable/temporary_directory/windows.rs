//! Windows temporary-directory parent discovery.

use super::push_unique_path;
use std::io;
use std::path::{Path, PathBuf};

pub(super) struct TemporaryParentCandidates {
    memory_backed: Vec<PathBuf>,
    fallback: Vec<PathBuf>,
}

pub(super) fn temporary_parent_candidates() -> TemporaryParentCandidates {
    let mut memory_backed = Vec::new();
    add_env_temporary_parent_candidates(
        &mut memory_backed,
        &["CANON_MEMORY_BACKED_TMPDIR", "RAMDISK", "RAMDISK_TMPDIR"],
    );
    let mut fallback = Vec::new();
    add_env_temporary_parent_candidates(&mut fallback, &["TMPDIR", "TEMP", "TMP"]);
    push_unique_path(&mut fallback, std::env::temp_dir());
    TemporaryParentCandidates {
        memory_backed,
        fallback,
    }
}

impl TemporaryParentCandidates {
    pub(super) fn memory_backed(&self) -> &[PathBuf] {
        &self.memory_backed
    }

    pub(super) fn fallback(&self) -> &[PathBuf] {
        &self.fallback
    }

    pub(super) fn allows_executables(&self, _parent: &Path) -> bool {
        true
    }
}

pub(super) fn canonical_temporary_parent(parent: &Path) -> io::Result<PathBuf> {
    Ok(parent.to_path_buf())
}

pub(super) fn resolve_standard_temporary_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

fn add_env_temporary_parent_candidates(parents: &mut Vec<PathBuf>, names: &[&str]) {
    for name in names {
        let Some(value) = std::env::var_os(name) else {
            continue;
        };
        if !value.is_empty() {
            push_unique_path(parents, PathBuf::from(value));
        }
    }
}
