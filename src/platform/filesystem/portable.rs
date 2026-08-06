mod directory_tree;
mod git_path;
mod materialize;
mod move_path;
mod permissions;
mod private_directory;
mod temporary_directory;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

pub(crate) use directory_tree::make_directory_tree_private;
pub(crate) use git_path::{git_path_bytes, os_string_from_bytes, path_from_git_stdout};
pub(crate) use materialize::{create_materialized_symlink, hardlink_file_or_copy_symlink};
pub(crate) use move_path::{mirror_evaluator_codex_home_file, move_path};
pub(crate) use permissions::{
    chmod_secret_dir_no_access, make_hook_executable, restore_secret_dir_mode, secret_dir_mode,
    set_materialized_dir_permissions, set_materialized_file_permissions, SecretDirMode,
};
pub(crate) use private_directory::{create_private_dir, create_private_dir_all};
pub(crate) use temporary_directory::{
    OwnedPrivateTemporaryDirectory, PrivateTemporaryDirectoryAllocator,
};

fn filesystem_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
