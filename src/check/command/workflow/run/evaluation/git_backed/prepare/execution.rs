use crate::check::command::workflow::failure::CheckPublicOutputProgress;
use crate::check::command::workflow::failure::{
    or_fail_at_selection_boundary, CheckFailureOutput, SelectionBoundary,
};
use crate::check::command::workflow::prepare::PreparedGitBackedCheckExecution;
use crate::check::command::{
    prepare_git_backed_check_execution, GitBackedCheckResources,
    PrepareGitBackedCheckExecutionOptions,
};
use crate::check::interrogation::state::CheckTreeContext;
use crate::check::CheckRunCaches;
use crate::cli::CommandError;
use crate::config_types::CheckConfig;
use crate::git::TreeSource;
use crate::logs::DiagnosticLogWriter;
use std::path::Path;

pub(super) struct GitBackedExecutionContext<'a> {
    pub(super) diagnostic_log: &'a mut DiagnosticLogWriter,
    pub(super) public_output_progress: &'a mut CheckPublicOutputProgress,
    pub(super) failure_output: &'a mut CheckFailureOutput,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_git_backed_execution(
    root: &Path,
    config: &CheckConfig,
    checked_tree: &TreeSource,
    tree_context: CheckTreeContext,
    no_sandbox: bool,
    resources: GitBackedCheckResources,
    check_caches: &CheckRunCaches,
    context: GitBackedExecutionContext<'_>,
) -> Result<PreparedGitBackedCheckExecution, CommandError> {
    or_fail_at_selection_boundary(
        prepare_git_backed_check_execution(
            root,
            config,
            PrepareGitBackedCheckExecutionOptions {
                tree_source: checked_tree,
                tree_context,
                no_sandbox,
                resources,
                repo_inspection: check_caches.repo_inspection.clone(),
                temporary_directory_allocator: &check_caches.temporary_directory_allocator,
            },
        ),
        SelectionBoundary::After,
        context.diagnostic_log,
        context.public_output_progress,
        context.failure_output,
    )
}
