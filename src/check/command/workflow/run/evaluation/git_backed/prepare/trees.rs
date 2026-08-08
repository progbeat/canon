use crate::check::command::workflow::prepare::resolve_git_backed_tree_state;
use crate::check::command::GitBackedCheckResources;
use crate::check::core::CheckCommandArgs;
use crate::check::interrogation::state::CheckTreeContext;
use crate::check::CheckRunCaches;
use crate::git::TreeSource;
use std::path::Path;

pub(super) struct ResolvedGitBackedCheckTrees {
    pub(super) checked_tree: TreeSource,
    pub(super) tree_context: CheckTreeContext,
}

pub(super) fn resolve_git_backed_check_trees(
    root: &Path,
    command: &CheckCommandArgs,
    check_caches: &mut CheckRunCaches,
    resources: &GitBackedCheckResources,
) -> Result<ResolvedGitBackedCheckTrees, String> {
    let tree_state = resolve_git_backed_tree_state(
        root,
        &command.tree,
        &command.against_tree,
        &mut check_caches.repo_inspection,
        resources,
    )?;
    let tree_context =
        tree_state.check_tree_context(root, &mut check_caches.visible_tree_oid_cache, resources)?;
    Ok(ResolvedGitBackedCheckTrees {
        checked_tree: tree_state.checked_tree,
        tree_context,
    })
}
