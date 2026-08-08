use super::fs::{inspect_hook_file, HookFile};
use super::{
    DEFAULT_GIT_HOOKS_PATH, DEFAULT_GIT_PRE_COMMIT_HOOK_COMMON_DIR_PATH,
    DEFAULT_GIT_PRE_COMMIT_HOOK_PATH,
};
use crate::git::{git_common_dir, git_config_get, is_git_worktree};
use std::path::{Component, Path, PathBuf};

fn effective_git_hooks_path_for_worktree(root: &Path) -> Result<Option<String>, String> {
    git_config_get(root, "core.hooksPath")
        .map_err(|err| format!("failed to read effective git core.hooksPath: {err}"))
}

pub(super) struct HookPreflight {
    pub(super) effective_git_hooks_path: Option<String>,
    pub(super) pre_commit_hook: HookFile,
    pub(super) is_git_worktree: bool,
    pub(super) root: PathBuf,
    pub(super) git_hooks_path: PathBuf,
    pub(super) pre_commit_hook_path: PathBuf,
}

impl HookPreflight {
    pub(super) fn load(root: &Path) -> Result<HookPreflight, String> {
        let is_git_worktree = is_git_worktree(root)?;
        let git_hooks_path = canon_git_hooks_path(root, is_git_worktree)?;
        let pre_commit_hook_path = canon_pre_commit_hook_path(root, is_git_worktree)?;
        let effective_git_hooks_path = if is_git_worktree {
            effective_git_hooks_path_for_worktree(root)?
        } else {
            None
        };
        Ok(HookPreflight {
            effective_git_hooks_path,
            pre_commit_hook: inspect_hook_file(&pre_commit_hook_path),
            is_git_worktree,
            root: root.to_path_buf(),
            git_hooks_path,
            pre_commit_hook_path,
        })
    }
}

fn canon_git_hooks_path(root: &Path, is_git_worktree: bool) -> Result<PathBuf, String> {
    if is_git_worktree {
        return Ok(git_common_dir(root)?.join("hooks"));
    }
    Ok(root.join(DEFAULT_GIT_HOOKS_PATH))
}

fn canon_pre_commit_hook_path(root: &Path, is_git_worktree: bool) -> Result<PathBuf, String> {
    if is_git_worktree {
        return Ok(git_common_dir(root)?.join(DEFAULT_GIT_PRE_COMMIT_HOOK_COMMON_DIR_PATH));
    }
    Ok(root.join(DEFAULT_GIT_PRE_COMMIT_HOOK_PATH))
}

pub(super) fn git_hooks_path_matches(root: &Path, expected: &Path, existing: &str) -> bool {
    let existing = Path::new(existing);
    let existing = if existing.is_absolute() {
        existing.to_path_buf()
    } else {
        root.join(existing)
    };
    let Some(existing) = normalize_path_without_parent_traversal(&existing) else {
        return false;
    };
    let Some(expected) = normalize_path_without_parent_traversal(expected) else {
        return false;
    };
    existing == expected
}

fn normalize_path_without_parent_traversal(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => return None,
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: gO,D8
    fn hooks_path_matching_rejects_parent_traversal_instead_of_reinterpreting_it() {
        let root = Path::new("repo");
        let expected = root.join(".git/hooks");

        assert!(git_hooks_path_matches(root, &expected, "./.git/./hooks"));
        assert!(!git_hooks_path_matches(
            root,
            &expected,
            "link/../.git/hooks"
        ));
    }
}
