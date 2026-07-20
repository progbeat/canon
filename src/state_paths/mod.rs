use crate::platform::path_from_git_stdout;
use crate::project::command_output_trimmed;
use std::path::{Path, PathBuf};
use std::process::Command;

// [1g,I4] CANON_STATE_DIR selects canon's cross-command output namespace.
// This path-only configuration is kept outside checked-project evaluation:
// it never provides tree contents, OIDs, diffs, scopes, cache eligibility, or
// evaluator context.
pub(crate) const CANON_STATE_DIR_GIT_PATH: &str = "canon";

#[derive(Clone)]
pub(crate) struct CanonStateRoot(PathBuf);

impl CanonStateRoot {
    // [1g] Commands that require persistent state use the environment value or
    // the exact Git-derived default from the glossary.
    pub(crate) fn resolve(root: &Path) -> Result<CanonStateRoot, String> {
        let path = match explicit_canon_state_root()? {
            Some(state_root) => state_root,
            None => resolve_glossary_default(root)?,
        };
        Ok(CanonStateRoot(path))
    }

    pub(crate) fn resolve_if_available(root: &Path) -> Result<Option<CanonStateRoot>, String> {
        if let Some(path) = explicit_canon_state_root()? {
            return Ok(Some(CanonStateRoot(path)));
        }
        // [1g,I4] This operation returns only the glossary-defined pathname for
        // canon-owned output. The path is opaque storage configuration; it
        // conveys no information about the checked state.
        match resolve_glossary_default(root) {
            Ok(path) => Ok(Some(CanonStateRoot(path))),
            Err(_) if crate::project::git_project_root(root).is_err() => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn resolve_explicit_if_configured() -> Result<Option<CanonStateRoot>, String> {
        explicit_canon_state_root().map(|path| path.map(CanonStateRoot))
    }

    pub(crate) fn join(&self, relative: &str) -> PathBuf {
        if relative.is_empty() {
            self.0.clone()
        } else {
            self.0.join(relative)
        }
    }
}

fn resolve_glossary_default(root: &Path) -> Result<PathBuf, String> {
    // [1g,I4] Use exactly the glossary's `git rev-parse --git-path canon`
    // expression. This is a path-location operation, not an inspection of
    // checked files, refs, trees, diffs, tracking, or repository status.
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--git-path", CANON_STATE_DIR_GIT_PATH])
        .output()
        .map_err(|err| format!("failed to resolve default CANON_STATE_DIR: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "failed to resolve default CANON_STATE_DIR: {}",
            command_output_trimmed(&output.stderr, "CANON_STATE_DIR resolver stderr")?
        ));
    }
    Ok(root.join(path_from_git_stdout(output.stdout)?))
}

pub(crate) fn explicit_canon_state_root() -> Result<Option<PathBuf>, String> {
    let Some(configured) = std::env::var_os("CANON_STATE_DIR") else {
        return Ok(None);
    };
    if configured.is_empty() {
        return Err("CANON_STATE_DIR must not be empty".to_string());
    }
    Ok(Some(PathBuf::from(configured)))
}

pub(crate) fn canon_state_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    Ok(CanonStateRoot::resolve(root)?.join(relative))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    // xpec: 1g,I4
    #[test]
    fn glossary_default_is_only_the_canon_owned_output_path() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("canon-state-path-{}-{unique}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let init = Command::new("git").arg("init").arg(&root).output().unwrap();
        assert!(
            init.status.success(),
            "{}",
            String::from_utf8_lossy(&init.stderr)
        );

        let resolved = resolve_glossary_default(&root).unwrap();

        assert_eq!(resolved, root.join(".git").join(CANON_STATE_DIR_GIT_PATH));
        let _ = fs::remove_dir_all(root);
    }
}
