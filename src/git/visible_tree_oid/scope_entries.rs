use super::hash::{scope_entry_path, visible_tree_oid_from_entries, GitObjectHashAlgorithm};
use crate::git::program::StagedTrackedFile;
use crate::scope::{path_bytes_in_scope, pathspec_is_exclude};

pub(super) fn visible_scope_entries_from_files(
    files: &[StagedTrackedFile],
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
    files: &[StagedTrackedFile],
    scope: &[String],
    object_hash_algorithm: GitObjectHashAlgorithm,
) -> Result<Option<String>, String> {
    // This presence check does not select visible tree entries. It only keeps
    // an explicit include pathspec that matches no checked Git path from being
    // treated as the Git empty tree.
    if !visible_scope_include_terms_are_present_in_checked_tree(files, scope)? {
        return Ok(None);
    }
    visible_tree_oid_from_files(files, scope, object_hash_algorithm).map(Some)
}

pub(super) fn visible_tree_oid_from_files(
    files: &[StagedTrackedFile],
    scope: &[String],
    object_hash_algorithm: GitObjectHashAlgorithm,
) -> Result<String, String> {
    let entries = visible_scope_entries_from_files(files, scope)?;
    visible_tree_oid_from_entries(&entries, object_hash_algorithm)
}

pub(super) fn visible_scope_include_terms_are_present_in_checked_tree(
    files: &[StagedTrackedFile],
    scope: &[String],
) -> Result<bool, String> {
    let include_pathspecs = scope
        .iter()
        .filter_map(|pathspec| match pathspec_is_exclude(pathspec) {
            Ok(true) => None,
            Ok(false) => Some(Ok(pathspec)),
            Err(err) => Some(Err(err)),
        })
        .collect::<Result<Vec<_>, String>>()?;
    if include_pathspecs.iter().any(|pathspec| pathspec == &".") {
        return Ok(true);
    }
    for pathspec in include_pathspecs {
        let pathspec_scope = [pathspec.clone()];
        let mut matched = false;
        for file in files {
            if path_bytes_in_scope(&file.path, &pathspec_scope)? {
                matched = true;
                break;
            }
        }
        if !matched {
            return Ok(false);
        }
    }
    Ok(true)
}

fn tracked_files_scope_entries(files: &[&StagedTrackedFile]) -> Vec<String> {
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
