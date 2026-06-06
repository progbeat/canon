#[path = "platform_windows/app_server.rs"]
mod app_server;
#[path = "platform_windows/chmod.rs"]
mod chmod;
#[path = "platform_windows/dirs.rs"]
mod dirs;
#[path = "platform_windows/git_path.rs"]
mod git_path;
#[path = "platform_windows/materialize.rs"]
mod materialize;
#[path = "platform_windows/move_path.rs"]
mod move_path;
#[path = "platform_windows/security.rs"]
mod security;
#[path = "platform_windows/snapshot.rs"]
mod snapshot;

pub(crate) use app_server::{
    install_check_signal_handlers, prepare_app_server_command, terminate_app_server_child,
};
pub(crate) use chmod::{
    chmod_secret_dir_no_access, make_hook_executable, restore_secret_dir_mode, secret_dir_mode,
    set_materialized_permissions, set_private_permissions, SecretDirMode,
};
pub(crate) use dirs::{create_private_dir, create_private_dir_all};
pub(crate) use git_path::{git_path_bytes, os_string_from_bytes, path_from_git_bytes};
pub(crate) use materialize::{create_materialized_symlink, hardlink_file_or_copy_symlink};
pub(crate) use move_path::{mirror_evaluator_codex_home_file, move_path};
pub(crate) use snapshot::{
    memory_backed_staged_snapshot_parent_candidates, ordinary_staged_snapshot_parent_candidates,
};
