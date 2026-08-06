use crate::check::core::ResolvedExpectation;
use crate::check::interrogation::state::CheckRuntime;
use crate::config_types::{AGAINST_TREE_DIFF_FROM, DEFAULT_DIFF_FROM};
use crate::evaluator::EvaluatorError;
use crate::git::VisibleTreeOidCache;
use crate::xpec_state::LastResult;
use std::path::Path;

pub(crate) struct ResolvedDiffFrom {
    pub(crate) tree_oid: Option<String>,
}

pub(crate) fn resolve_diff_from(
    runtime: &CheckRuntime<'_>,
    expectation: &ResolvedExpectation,
    last_pass: Option<&LastResult>,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<ResolvedDiffFrom, EvaluatorError> {
    if runtime.is_in_place() {
        // In-place mode has no Git-backed diff base. Its resolved expectations
        // are validated before interrogation, and prompt rendering clears any
        // diff-only turn inputs for this mode.
        return Ok(ResolvedDiffFrom { tree_oid: None });
    }
    // `:checkpoint` uses the last pass checked tree only while that tree
    // exists; `:against-tree` uses the against tree; other values were resolved
    // from the same TreeSource contract during check preparation.
    let diff_from = expectation.diff_from.as_str();
    if diff_from == DEFAULT_DIFF_FROM {
        let against_tree_oid = runtime
            .git_against_tree_oid()
            .ok_or_else(|| EvaluatorError::message("Git-backed check has no against tree OID"))?;
        return checkpoint_diff_base(
            runtime.root,
            last_pass,
            against_tree_oid,
            visible_tree_oid_cache,
        );
    }
    if diff_from == AGAINST_TREE_DIFF_FROM {
        return Ok(ResolvedDiffFrom {
            tree_oid: runtime.git_against_tree_oid().map(str::to_string),
        });
    }
    Ok(ResolvedDiffFrom {
        tree_oid: Some(
            runtime
                .explicit_diff_from_tree_oid(diff_from)
                .ok_or_else(|| {
                    EvaluatorError::message(format!(
                        "diff-from was not resolved during preparation: {}",
                        diff_from
                    ))
                })?
                .to_string(),
        ),
    })
}

pub(super) fn checkpoint_diff_base(
    root: &Path,
    last_pass: Option<&LastResult>,
    against_tree_oid: &str,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<ResolvedDiffFrom, EvaluatorError> {
    if let Some(checked_tree_oid) =
        last_pass.and_then(|last_pass| last_pass.checked_tree_oid.as_deref())
    {
        if crate::git::git_object_oid_has_known_shape(checked_tree_oid)
            && visible_tree_oid_cache
                .git_tree_object_exists(root, checked_tree_oid)
                .map_err(EvaluatorError::message)?
        {
            return Ok(ResolvedDiffFrom {
                tree_oid: Some(checked_tree_oid.to_string()),
            });
        }
    }
    Ok(ResolvedDiffFrom {
        tree_oid: Some(against_tree_oid.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::checkpoint_diff_base;
    use crate::git::VisibleTreeOidCache;
    use crate::xpec_state::{LastResult, LastResultResponse, LastResultStatus};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: gO
    fn uses_existing_checkpoint_tree() {
        let root = git_project("checkpoint-existing");
        let checked_tree_oid = crate::git::TreeSource::Staged
            .tree_oid_for_prompt_diff(&root)
            .unwrap();
        let last_pass = last_pass_with_checked_tree_oid(&checked_tree_oid);

        let resolved = checkpoint_diff_base(
            &root,
            Some(&last_pass),
            "against-tree",
            &mut VisibleTreeOidCache::new(),
        )
        .unwrap();

        assert_eq!(resolved.tree_oid, Some(checked_tree_oid));
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: gO
    fn ignores_missing_checkpoint_tree() {
        let root = git_project("checkpoint-missing");
        let last_pass = last_pass_with_checked_tree_oid("ffffffffffffffffffffffffffffffffffffffff");

        let resolved = checkpoint_diff_base(
            &root,
            Some(&last_pass),
            "against-tree",
            &mut VisibleTreeOidCache::new(),
        )
        .unwrap();

        assert_eq!(resolved.tree_oid.as_deref(), Some("against-tree"));
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: gO
    fn ignores_non_oid_checkpoint_tree() {
        let root = git_project("checkpoint-revspec");
        let last_pass = last_pass_with_checked_tree_oid("HEAD^{tree}");

        let resolved = checkpoint_diff_base(
            &root,
            Some(&last_pass),
            "against-tree",
            &mut VisibleTreeOidCache::new(),
        )
        .unwrap();

        assert_eq!(resolved.tree_oid.as_deref(), Some("against-tree"));
        let _ = fs::remove_dir_all(root);
    }

    fn last_pass_with_checked_tree_oid(checked_tree_oid: &str) -> LastResult {
        LastResult {
            response_timestamp: "1970-01-01T00:00:00Z".to_string(),
            updated_timestamp: "1970-01-01T00:00:00Z".to_string(),
            status: LastResultStatus::Pass,
            response: LastResultResponse::answered(
                "yes",
                "`src/main.rs`",
                Some(vec![".".to_string()]),
            ),
            q_scope: vec![".".to_string()],
            visible_scope: vec![".".to_string()],
            checked_tree_oid: Some(checked_tree_oid.to_string()),
            visible_tree_oid: None,
            diff_from: None,
            diff_from_tree_oid: None,
        }
    }

    fn git_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("canon-{name}-{}-{unique}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        git(&root, &["init"]);
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        git(&root, &["add", "src/main.rs"]);
        root
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        // xpec: gO
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
