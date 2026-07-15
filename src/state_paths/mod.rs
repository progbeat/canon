use std::path::{Path, PathBuf};

// Canon-owned persistent state is rooted at CANON_STATE_DIR when configured,
// and otherwise at `git rev-parse --git-path canon` (or `./canon` for a
// non-Git in-place project).
pub(crate) const CANON_STATE_DIR_GIT_PATH: &str = "canon";

pub(crate) fn canon_state_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let state_root = if let Some(configured) = std::env::var_os("CANON_STATE_DIR") {
        if configured.is_empty() {
            return Err("CANON_STATE_DIR must not be empty".to_string());
        }
        PathBuf::from(configured)
    } else {
        match crate::git::resolve_git_path(root, CANON_STATE_DIR_GIT_PATH) {
            Ok(path) => path,
            Err(_error) if crate::project::git_project_root(root).is_err() => {
                root.join(CANON_STATE_DIR_GIT_PATH)
            }
            Err(error) => return Err(error),
        }
    };
    if relative.is_empty() {
        Ok(state_root)
    } else {
        Ok(state_root.join(relative))
    }
}
