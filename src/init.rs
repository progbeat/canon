use crate::check::CHECK_PATH;
use crate::fs_util::{ensure_project_dir_without_symlinks, path_exists_no_follow, write_new_file};
use crate::output::write_stdout_line;
use std::path::Path;

const DEFAULT_CHECK_CONFIG_TEMPLATE_FILE_CONTENTS: &str =
    include_str!("../.canon/templates/default/check.yml");

pub(crate) fn run_init(root: &Path) -> Result<(), String> {
    let check_path = root.join(CHECK_PATH);
    if path_exists_no_follow(&check_path)? {
        return Err(format!("{} already exists", CHECK_PATH));
    }

    // These are user-owned project configuration files, not canon runtime
    // state: they live in the worktree so humans can review and version them.
    if let Some(parent) = check_path.parent() {
        ensure_project_dir_without_symlinks(root, parent)?;
    }
    write_new_file(&check_path, DEFAULT_CHECK_CONFIG_TEMPLATE_FILE_CONTENTS)?;
    // This success line becomes eligible only after the config file exists;
    // `write_stdout_line` flushes it immediately and no later init work remains.
    write_stdout_line(&format!("Created {}", CHECK_PATH))?;
    Ok(())
}
