use crate::app::LazyAppServerRunner;
use crate::check::config::validation::check_config_loads_plugins;
use crate::check::interrogation::state::CheckTreeContext;
use crate::config_types::CheckConfig;
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::staged::StagedWorktreeView;
use std::path::Path;

pub(crate) struct PreparedGitBackedCheckExecution {
    pub(crate) staged_view: StagedWorktreeView,
    pub(crate) tree_source: TreeSource,
    pub(crate) tree_context: CheckTreeContext,
    pub(crate) runner: LazyAppServerRunner,
}

pub(crate) struct PrepareGitBackedCheckExecutionOptions<'a> {
    pub(crate) tree_source: &'a TreeSource,
    pub(crate) against_tree: &'a TreeSource,
    pub(crate) no_sandbox: bool,
    pub(crate) storage: GitBackedCheckStorage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitBackedCheckStorage {
    Persistent,
    InvocationLocal,
}

pub(crate) fn prepare_git_backed_check_execution(
    root: &Path,
    config: &CheckConfig,
    options: PrepareGitBackedCheckExecutionOptions<'_>,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<PreparedGitBackedCheckExecution, String> {
    let staged_view = StagedWorktreeView::apply_for_tree_source(root, options.tree_source.clone())?;
    // Prompt rendering receives concrete checked/against tree OIDs, so
    // non-staged `--tree` checks still show the selected checked-vs-against
    // diff.
    let tree_context = CheckTreeContext {
        checked_tree_oid: options.tree_source.tree_oid_for_prompt_diff(root)?,
        against_tree_oid: options.against_tree.tree_oid_for_prompt_diff(root)?,
        checked_file_count: visible_tree_oid_cache.checked_file_count(root, options.tree_source)?,
    };
    let load_plugins = check_config_loads_plugins(config);
    let runner = match options.storage {
        GitBackedCheckStorage::Persistent => {
            LazyAppServerRunner::new(root, load_plugins, &config.agent, options.no_sandbox)?
        }
        GitBackedCheckStorage::InvocationLocal => LazyAppServerRunner::new_without_state(
            root,
            load_plugins,
            &config.agent,
            options.no_sandbox,
        ),
    };
    Ok(PreparedGitBackedCheckExecution {
        staged_view,
        tree_source: options.tree_source.clone(),
        tree_context,
        runner,
    })
}
