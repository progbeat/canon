use crate::fs_util::ensure_dir_without_symlinks;
use crate::git::git_project_root;
use crate::output::write_stdout_line;
use crate::project_types::Config;
use std::env;
use std::path::{Path, PathBuf};

pub(crate) fn print_root(config: &Config) -> Result<(), String> {
    ensure_dir_without_symlinks(&config.root)?;
    write_stdout_line(&config.root.display().to_string())
}

impl Config {
    pub(crate) fn from_env() -> Result<Config, String> {
        let thread_id = env::var("CODEX_THREAD_ID")
            .map_err(|_| "CODEX_THREAD_ID is required in v1".to_string())?;
        if thread_id.trim().is_empty() {
            return Err("CODEX_THREAD_ID is empty".to_string());
        }
        if thread_id == "."
            || thread_id == ".."
            || thread_id.contains('/')
            || thread_id.contains('\\')
        {
            return Err("CODEX_THREAD_ID must be a single path segment".to_string());
        }

        let current_dir =
            env::current_dir().map_err(|err| format!("failed to read current dir: {}", err))?;
        let project_root = git_project_root(&current_dir)?;
        Config::for_project_thread(&project_root, &thread_id)
    }

    pub(crate) fn for_project_thread(root: &Path, thread_id: &str) -> Result<Config, String> {
        let state_root = crate::state_paths::canon_state_path(root, "")?;
        // Notes are intentionally thread-scoped retained data: CODEX_THREAD_ID
        // selects a retained-data key, not a cache generation created by every
        // command run. Thus the project's bounded-retained-data premise bounds
        // the set of these roots even when callers switch between thread IDs.
        // Appends under each retained root are small note/index log records, and
        // those logs are threshold-compacted after enough appended bytes
        // accumulate to pay for each rewrite. Across N appended bytes, the
        // notes component therefore writes O(N) bytes. Automatic cache cleanup
        // must not delete these user-retained notes.
        Ok(Config {
            root: state_root
                .join(crate::state_paths::RETAINED_CODEX_DATA_DIR_NAME)
                .join(thread_id),
        })
    }
}

pub(crate) fn project_root_or_current(start: &Path) -> Result<PathBuf, String> {
    match git_project_root(start) {
        Ok(root) => Ok(root),
        Err(_) => env::current_dir().map_err(|err| format!("failed to read current dir: {}", err)),
    }
}
