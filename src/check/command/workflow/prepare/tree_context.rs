use super::GitBackedCheckResources;
use crate::check::interrogation::state::CheckTreeContext;
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::repo_inspection::RepoInspectionCache;
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) struct ResolvedGitBackedTreeState {
    pub(crate) checked_tree: TreeSource,
    pub(crate) checked_tree_oid: String,
    pub(crate) against_tree_oid: String,
    pub(crate) head_tree_oid: Option<String>,
}

impl ResolvedGitBackedTreeState {
    pub(crate) fn check_tree_context(
        &self,
        root: &Path,
        visible_tree_oid_cache: &mut VisibleTreeOidCache,
        resources: &GitBackedCheckResources,
    ) -> Result<CheckTreeContext, String> {
        Ok(CheckTreeContext {
            checked_tree_oid: self.checked_tree_oid.clone(),
            against_tree_oid: self.against_tree_oid.clone(),
            head_tree_oid: self.head_tree_oid.clone(),
            explicit_diff_from_tree_oids: BTreeMap::new(),
            checked_file_count: visible_tree_oid_cache
                .checked_file_count(root, &self.checked_tree)?,
            prompt_git_environment: resources.prompt_git_environment(),
        })
    }
}

pub(crate) fn resolve_git_backed_tree_state(
    root: &Path,
    checked_tree_value: &str,
    against_tree_value: &str,
    repo_inspection: &mut RepoInspectionCache,
    resources: &GitBackedCheckResources,
) -> Result<ResolvedGitBackedTreeState, String> {
    let (checked_tree, against_tree) = repo_inspection.resolve_checked_and_against_trees(
        root,
        checked_tree_value,
        against_tree_value,
    )?;
    resolve_git_backed_tree_state_from_sources(
        root,
        checked_tree,
        &against_tree,
        repo_inspection,
        resources,
    )
}

fn resolve_git_backed_tree_state_from_sources(
    root: &Path,
    checked_tree: TreeSource,
    against_tree: &TreeSource,
    repo_inspection: &mut RepoInspectionCache,
    resources: &GitBackedCheckResources,
) -> Result<ResolvedGitBackedTreeState, String> {
    let checked_tree = resources.freeze_tree_source(root, checked_tree)?;
    let against_tree = resources.freeze_tree_source(root, against_tree.clone())?;
    // Prompt rendering and feedback receive concrete checked/against tree OIDs,
    // so non-staged `--tree` checks use the selected checked-vs-against state.
    let against_tree_oid = against_tree.resolved_tree_oid()?.to_string();
    let head_tree_oid = if !resources.persists_failure_history() {
        None
    } else if matches!(
        &against_tree,
        TreeSource::DefaultAgainstHead { .. } | TreeSource::DefaultAgainstUnbornHead { .. }
    ) {
        Some(against_tree_oid.clone())
    } else {
        let head = repo_inspection
            .resolve_default_against_tree(root, crate::git::DEFAULT_AGAINST_TREE_ARG)?;
        let head = resources.freeze_tree_source(root, head)?;
        Some(head.resolved_tree_oid()?.to_string())
    };
    let checked_tree_oid = checked_tree.resolved_tree_oid()?.to_string();
    Ok(ResolvedGitBackedTreeState {
        checked_tree_oid,
        checked_tree,
        against_tree_oid,
        head_tree_oid,
    })
}

pub(crate) fn resolve_explicit_diff_from_tree_oids<'a>(
    root: &Path,
    diff_from_values: impl IntoIterator<Item = &'a str>,
    repo_inspection: &mut RepoInspectionCache,
    resources: &GitBackedCheckResources,
) -> Result<BTreeMap<String, String>, String> {
    let mut resolved = BTreeMap::new();
    for diff_from in diff_from_values {
        if matches!(
            diff_from,
            crate::config_types::DEFAULT_DIFF_FROM | crate::config_types::AGAINST_TREE_DIFF_FROM
        ) || resolved.contains_key(diff_from)
        {
            continue;
        }
        let source = repo_inspection.resolve_tree(root, diff_from, "diff-from")?;
        let source = resources.freeze_tree_source(root, source)?;
        let tree_oid = source.resolved_tree_oid()?.to_string();
        resolved.insert(diff_from.to_string(), tree_oid);
    }
    Ok(resolved)
}
