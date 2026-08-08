use super::{
    hash::{scope_entry_path, visible_tree_oid_from_entries, GitObjectHashAlgorithm},
    TrackedFile,
};
use crate::scope::{path_bytes_in_scope, pathspec_is_exclude};

pub(super) fn visible_scope_entries_from_files(
    files: &[TrackedFile],
    scope: &[String],
) -> Result<Vec<String>, String> {
    // This is the visible tree entry selection step: apply the complete
    // visible-scope pathspec to the checked Git tree's tracked files.
    let mut visible_files = Vec::new();
    for file in files {
        if path_bytes_in_scope(&file.path, scope)? {
            visible_files.push(file);
        }
    }
    Ok(tracked_files_scope_entries(&visible_files))
}

pub(super) fn visible_tree_oid_from_files_if_scope_present(
    files: &[TrackedFile],
    scope: &[String],
    object_hash_algorithm: GitObjectHashAlgorithm,
) -> Result<Option<String>, String> {
    // This presence check does not select visible tree entries. It only keeps
    // an explicit include pathspec that matches no checked Git path from being
    // treated as the Git empty tree.
    if !visible_scope_has_present_include_term(files, scope)? {
        return Ok(None);
    }
    visible_tree_oid_from_files(files, scope, object_hash_algorithm).map(Some)
}

pub(super) fn visible_tree_oid_from_files(
    files: &[TrackedFile],
    scope: &[String],
    object_hash_algorithm: GitObjectHashAlgorithm,
) -> Result<String, String> {
    let entries = visible_scope_entries_from_files(files, scope)?;
    visible_tree_oid_from_entries(&entries, object_hash_algorithm)
}

fn visible_scope_has_present_include_term(
    files: &[TrackedFile],
    scope: &[String],
) -> Result<bool, String> {
    let mut has_include_term = false;
    for pathspec in scope {
        if pathspec_is_exclude(pathspec)? {
            continue;
        }
        has_include_term = true;
        if pathspec == "." {
            return Ok(true);
        }
        let pathspec_scope = std::slice::from_ref(pathspec);
        for file in files {
            if path_bytes_in_scope(&file.path, pathspec_scope)? {
                return Ok(true);
            }
        }
    }
    // With only exclusions, Git's implicit include set is the whole tree.
    Ok(!has_include_term)
}

fn tracked_files_scope_entries(files: &[&TrackedFile]) -> Vec<String> {
    let mut entries = files
        .iter()
        .map(|file| {
            format!(
                "{} {}\t{}",
                file.mode,
                file.object_id,
                scope_entry_path(&file.path)
            )
        })
        .collect::<Vec<_>>();
    sort_scope_entries(&mut entries);
    entries
}

fn sort_scope_entries(entries: &mut Vec<String>) {
    entries.sort();
    entries.dedup();
}
