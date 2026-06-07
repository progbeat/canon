use super::hash::{
    parse_visible_tree_entry, scope_entry_path, visible_tree_oid_from_entries,
    GitObjectHashAlgorithm, RAW_PATH_HEX_PREFIX,
};
use super::scope_entries::{scope_includes_match_tracked_files, visible_scope_entries_from_files};
use crate::git::program::StagedTrackedFile;

#[test]
fn parent_scope_matches_raw_hex_child_entry() {
    let entry = format!(
        "100644 0123456789012345678901234567890123456789\t{}",
        scope_entry_path(b"dir/nonutf8-\xff.txt")
    );

    assert!(entry.contains(RAW_PATH_HEX_PREFIX));
    assert_eq!(
        parse_visible_tree_entry(&entry).unwrap().path,
        vec![b"dir".to_vec(), b"nonutf8-\xff.txt".to_vec()]
    );
}

#[test]
fn visible_scope_entries_are_selected_by_pathspec_without_blob_filtering() {
    let files = vec![StagedTrackedFile {
        path: b"deps/example".to_vec(),
        mode: "160000".to_string(),
        object_id: "0123456789012345678901234567890123456789".to_string(),
    }];

    let entries = visible_scope_entries_from_files(&files, &["deps".to_string()]).unwrap();

    assert_eq!(
        entries,
        vec!["160000 0123456789012345678901234567890123456789\tdeps/example"]
    );
    visible_tree_oid_from_entries(&entries, GitObjectHashAlgorithm::Sha1).unwrap();
}

#[test]
fn explicit_absent_scope_does_not_match_empty_tree() {
    let files = vec![StagedTrackedFile {
        path: b"src/check/run/run.rs".to_vec(),
        mode: "100644".to_string(),
        object_id: "0123456789012345678901234567890123456789".to_string(),
    }];

    assert!(scope_includes_match_tracked_files(&files, &["src/check/run".to_string()]).unwrap());
    assert!(
        !scope_includes_match_tracked_files(&files, &["src/check/run.rs".to_string()]).unwrap()
    );
    assert!(scope_includes_match_tracked_files(&[], &[".".to_string()]).unwrap());
}
