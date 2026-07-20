use crate::app::LazyAppServerRunner;
use crate::check::config::validation::check_config_loads_plugins;
use crate::check::interrogation::state::CheckTreeContext;
use crate::config_types::CheckConfig;
use crate::git::{GitPromptObjectArtifacts, TreeSource, VisibleTreeOidCache};
use crate::staged::StagedWorktreeView;
use std::path::Path;

pub(crate) struct PreparedGitBackedCheckExecution {
    pub(crate) staged_view: StagedWorktreeView,
    pub(crate) tree_source: TreeSource,
    pub(crate) tree_context: CheckTreeContext,
    pub(crate) runner: LazyAppServerRunner,
    _resources: GitBackedCheckResources,
}

pub(crate) struct PrepareGitBackedCheckExecutionOptions<'a> {
    pub(crate) tree_source: &'a TreeSource,
    pub(crate) tree_context: CheckTreeContext,
    pub(crate) no_sandbox: bool,
    pub(crate) resources: GitBackedCheckResources,
}

pub(crate) enum GitBackedCheckResources {
    Persistent,
    TemporaryQuery(GitPromptObjectArtifacts),
}

impl GitBackedCheckResources {
    pub(crate) fn temporary_query(root: &Path) -> Result<GitBackedCheckResources, String> {
        GitPromptObjectArtifacts::new(root).map(GitBackedCheckResources::TemporaryQuery)
    }

    fn tree_oid_for_prompt_diff(&self, root: &Path, source: &TreeSource) -> Result<String, String> {
        match self {
            GitBackedCheckResources::Persistent => source.tree_oid_for_prompt_diff(root),
            GitBackedCheckResources::TemporaryQuery(artifacts) => {
                source.tree_oid_for_temporary_prompt_diff(root, artifacts)
            }
        }
    }

    fn prompt_git_environment(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        match self {
            GitBackedCheckResources::Persistent => Vec::new(),
            GitBackedCheckResources::TemporaryQuery(artifacts) => artifacts.prompt_environment(),
        }
    }
}

pub(crate) fn prepare_git_backed_check_execution(
    root: &Path,
    config: &CheckConfig,
    options: PrepareGitBackedCheckExecutionOptions<'_>,
) -> Result<PreparedGitBackedCheckExecution, String> {
    // [ig,Ky] Both lifetimes use the hardlink policy's exact tmp_dir
    // selection. Persistent checks retain shared cache entries. Temporary
    // queries remove an owned root or transactionally restore a preexisting
    // caller root, so the same policy path never becomes persistent ask state.
    let staged_view = match &options.resources {
        GitBackedCheckResources::Persistent => {
            StagedWorktreeView::apply_for_tree_source(root, options.tree_source.clone())?
        }
        GitBackedCheckResources::TemporaryQuery(_) => {
            StagedWorktreeView::apply_invocation_local_for_tree_source(
                root,
                options.tree_source.clone(),
            )?
        }
    };
    let load_plugins = check_config_loads_plugins(config);
    let runner = match &options.resources {
        GitBackedCheckResources::Persistent => {
            LazyAppServerRunner::new(root, load_plugins, &config.agent, options.no_sandbox)?
        }
        GitBackedCheckResources::TemporaryQuery(_) => LazyAppServerRunner::new_without_state(
            root,
            load_plugins,
            &config.agent,
            options.no_sandbox,
        ),
    };
    Ok(PreparedGitBackedCheckExecution {
        staged_view,
        tree_source: options.tree_source.clone(),
        tree_context: options.tree_context,
        runner,
        _resources: options.resources,
    })
}

pub(crate) fn resolve_git_backed_check_tree_context(
    root: &Path,
    tree_source: &TreeSource,
    against_tree: &TreeSource,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    resources: &GitBackedCheckResources,
) -> Result<CheckTreeContext, String> {
    // Prompt rendering and feedback receive concrete checked/against tree OIDs,
    // so non-staged `--tree` checks use the selected checked-vs-against state.
    Ok(CheckTreeContext {
        checked_tree_oid: resources.tree_oid_for_prompt_diff(root, tree_source)?,
        against_tree_oid: resources.tree_oid_for_prompt_diff(root, against_tree)?,
        checked_file_count: visible_tree_oid_cache.checked_file_count(root, tree_source)?,
        prompt_git_environment: resources.prompt_git_environment(),
    })
}
