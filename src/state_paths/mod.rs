use crate::git::{git_project_root, resolve_git_path};
use std::path::{Path, PathBuf};

// [1g,90] CANON_STATE_DIR selects canon's cross-command output namespace.
// This path-only configuration is kept outside checked-project evaluation:
// it never provides tree contents, OIDs, diffs, scopes, cache eligibility, or
// evaluator context.
pub(crate) const CANON_STATE_DIR_GIT_PATH: &str = "canon";

// [fh] This is the complete CANON_STATE_DIR layout. Each growing family has
// one explicit retention owner:
// - `xpecs`: every normal check calls
//   `XpecStateCache::retain_only_current_configuration` before evaluation,
//   enabling later state writes only for the complete bounded configuration.
//   Git-backed checks remove absent per-ID directories; in-place checks remove
//   absent canonical Last Results while preserving at most one bounded
//   Git-backed gate cache per ID from the last Git-backed configuration;
// - `failure-history.jsonl`: `append_check_failure_history` retains 64 records
//   and `maybe_compact_failure_history` bounds obsolete appended bytes;
// - `logs`: `PersistentRuntimeLogHistory::{activate, write_event}` enforce the
//   configured byte limit over the whole directory;
// - `codex`: caller-selected notes covered by the bounded-retained-data
//   premise, with append logs threshold-compacted by their notes owner.
// The runtime-log lock is one fixed sibling file, not a growing family.
pub(crate) const XPEC_STATE_DIR_NAME: &str = "xpecs";
pub(crate) const FAILURE_HISTORY_FILE_NAME: &str = "failure-history.jsonl";
pub(crate) const RUNTIME_LOG_DIR_NAME: &str = "logs";
pub(crate) const RETAINED_CODEX_DATA_DIR_NAME: &str = "codex";

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
        // [1g,90] This operation returns only the glossary-defined pathname for
        // canon-owned output. The path is opaque storage configuration; it
        // conveys no information about the checked state.
        match resolve_glossary_default(root) {
            Ok(path) => Ok(Some(CanonStateRoot(path))),
            Err(_) if git_project_root(root).is_err() => Ok(None),
            Err(error) => Err(error),
        }
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
    // [1g,90] Use exactly the glossary's `git rev-parse --git-path canon`
    // expression. This is a path-location operation, not an inspection of
    // checked files, refs, trees, diffs, tracking, or repository status.
    resolve_git_path(root, CANON_STATE_DIR_GIT_PATH)
        .map_err(|err| format!("failed to resolve default CANON_STATE_DIR: {err}"))
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
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    // xpec: 1g,90
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

        let resolved = CanonStateRoot::resolve(&root).unwrap().join("");

        assert_eq!(resolved, root.join(".git").join(CANON_STATE_DIR_GIT_PATH));
        let _ = fs::remove_dir_all(root);
    }
}
