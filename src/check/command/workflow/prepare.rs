use crate::app::LazyAppServerRunner;
use crate::check::config::validation::check_config_loads_plugins;
use crate::check::interrogation::state::CheckTreeContext;
use crate::config_types::CheckConfig;
use crate::evaluator::EvaluatorProcessIsolation;
use crate::git::TreeSource;
use crate::materialization::TreeMaterializer;
use crate::repo_inspection::RepoInspectionCache;
use std::path::Path;

mod resources;
mod tree_context;

use resources::GitBackedCheckResourceKind;
pub(crate) use resources::GitBackedCheckResources;
pub(crate) use tree_context::{
    resolve_explicit_diff_from_tree_oids, resolve_git_backed_tree_state,
};

pub(crate) struct PreparedGitBackedCheckExecution {
    pub(crate) tree_materializer: TreeMaterializer,
    pub(crate) tree_source: TreeSource,
    pub(crate) tree_context: CheckTreeContext,
    pub(crate) runner: LazyAppServerRunner,
    pub(crate) resources: GitBackedCheckResources,
}

pub(crate) struct PrepareGitBackedCheckExecutionOptions<'a> {
    pub(crate) tree_source: &'a TreeSource,
    pub(crate) tree_context: CheckTreeContext,
    pub(crate) no_sandbox: bool,
    pub(crate) resources: GitBackedCheckResources,
    pub(crate) repo_inspection: RepoInspectionCache,
    pub(crate) temporary_directory_allocator:
        &'a crate::platform::filesystem::PrivateTemporaryDirectoryAllocator,
}

pub(crate) fn prepare_git_backed_check_execution(
    root: &Path,
    config: &CheckConfig,
    options: PrepareGitBackedCheckExecutionOptions<'_>,
) -> Result<PreparedGitBackedCheckExecution, String> {
    // [1t,g2,l] Both lifetimes use the hardlink policy's exact tmp_dir
    // selection. Its filesystem-shaped checked project is evaluator input
    // required by the materialization contract, not stored command state.
    // Persistent checks retain shared cache entries. A temporary query keeps
    // its command state and rollback journal in memory, then removes an owned
    // input root or restores a preexisting caller root.
    let tree_materializer = match &options.resources.kind {
        GitBackedCheckResourceKind::Persistent => {
            TreeMaterializer::apply_for_tree_source_with_repo_inspection_cache(
                root,
                options.tree_source.clone(),
                options.repo_inspection.clone(),
                options.temporary_directory_allocator,
            )?
        }
        GitBackedCheckResourceKind::TemporaryQuery(_) => {
            TreeMaterializer::apply_temporary_query_for_tree_source_with_repo_inspection_cache(
                root,
                options.tree_source.clone(),
                options.repo_inspection.clone(),
                options.temporary_directory_allocator,
            )?
        }
    };
    let load_plugins = check_config_loads_plugins(config);
    let process_isolation = if options.no_sandbox {
        EvaluatorProcessIsolation::ExternallyManaged
    } else {
        EvaluatorProcessIsolation::CanonManaged
    };
    let runner = match &options.resources.kind {
        GitBackedCheckResourceKind::Persistent => {
            LazyAppServerRunner::new(root, load_plugins, &config.agent, process_isolation)?
        }
        GitBackedCheckResourceKind::TemporaryQuery(_) => LazyAppServerRunner::new_temporary_query(
            root,
            load_plugins,
            &config.agent,
            process_isolation,
        )?,
    };
    Ok(PreparedGitBackedCheckExecution {
        tree_materializer,
        tree_source: options.tree_source.clone(),
        tree_context: options.tree_context,
        runner,
        resources: options.resources,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: Tv
    fn temporary_query_uses_its_frozen_private_tree_after_the_index_changes() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "canon-prompt-tree-oid-cache-{}-{unique}",
            process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        run_git(&root, &["init", "--quiet"]);
        fs::write(root.join("file.txt"), "first\n").unwrap();
        run_git(&root, &["add", "file.txt"]);
        let resources = GitBackedCheckResources::temporary_query(
            &root,
            &crate::platform::filesystem::PrivateTemporaryDirectoryAllocator::new(),
        )
        .unwrap();
        let source = resources
            .freeze_tree_source(&root, TreeSource::Staged)
            .unwrap();

        fs::write(root.join("file.txt"), "second\n").unwrap();
        run_git(&root, &["add", "file.txt"]);
        let mut repo_inspection = RepoInspectionCache::new();

        assert_eq!(
            repo_inspection
                .tree_file_content(&root, &source, "file.txt")
                .unwrap(),
            "first\n"
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn run_git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success()); // xpec: d
    }
}
