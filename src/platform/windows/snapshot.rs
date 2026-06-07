use crate::platform::push_unique_path;
use std::path::PathBuf;

pub(crate) fn memory_backed_staged_snapshot_parent_candidates() -> Vec<PathBuf> {
    let mut parents = Vec::new();
    add_env_staged_snapshot_parent_candidates(
        &mut parents,
        &["CANON_MEMORY_BACKED_TMPDIR", "RAMDISK", "RAMDISK_TMPDIR"],
    );
    parents
}

pub(crate) fn ordinary_staged_snapshot_parent_candidates() -> Vec<PathBuf> {
    let mut parents = Vec::new();
    add_env_staged_snapshot_parent_candidates(&mut parents, &["TMPDIR", "TEMP", "TMP"]);
    push_unique_path(&mut parents, std::env::temp_dir());
    parents
}

fn add_env_staged_snapshot_parent_candidates(parents: &mut Vec<PathBuf>, names: &[&str]) {
    for name in names {
        let Some(value) = std::env::var_os(name) else {
            continue;
        };
        if !value.is_empty() {
            push_unique_path(parents, PathBuf::from(value));
        }
    }
}
