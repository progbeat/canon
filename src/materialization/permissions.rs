//! Permission handling for extracted and materialized Git-tree entries.

use crate::platform::filesystem;
use std::path::Path;

pub(super) fn remove_write_permissions_from_materialized_dir(path: &Path) -> Result<(), String> {
    filesystem::set_materialized_dir_permissions(path)
}

pub(super) fn remove_write_permissions_from_extracted_file(
    path: &Path,
    mode: &str,
) -> Result<(), String> {
    filesystem::set_materialized_file_permissions(path, mode)
}

pub(super) fn make_materialization_tree_private(path: &Path) -> Result<(), String> {
    // [1t] Materialized regular files are hardlinks to read-only lazy entries.
    // Re-open only directories for removal; changing a file mode here would
    // also make its cached lazy inode writable.
    filesystem::make_directory_tree_private(path)
}
