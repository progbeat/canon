use super::failure::write_check_error_finish_event;
use crate::app::LazyAppServerRunner;
use crate::check::config::validation::check_config_loads_plugins;
use crate::check::interrogation::state::CheckTreeContext;
use crate::config_types::CheckConfig;
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::logs::DiagnosticLogWriter;
use crate::staged::StagedWorktreeView;
use std::path::Path;

pub(crate) struct PreparedCheckExecution {
    pub(crate) staged_view: StagedWorktreeView,
    pub(crate) tree_source: TreeSource,
    pub(crate) tree_context: CheckTreeContext,
    pub(crate) runner: LazyAppServerRunner,
}

pub(crate) struct PrepareCheckExecutionOptions<'a> {
    pub(crate) tree_source: &'a TreeSource,
    pub(crate) against_tree: &'a TreeSource,
    pub(crate) no_sandbox: bool,
    pub(crate) query: bool,
    pub(crate) errors_on_failure: usize,
}

pub(crate) fn prepare_check_execution(
    root: &Path,
    config: &CheckConfig,
    diagnostic_log: &mut DiagnosticLogWriter,
    options: PrepareCheckExecutionOptions<'_>,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<PreparedCheckExecution, String> {
    let staged_view =
        match StagedWorktreeView::apply_for_tree_source(root, options.tree_source.clone()) {
            Ok(staged_view) => staged_view,
            Err(err) => {
                write_prepare_check_failure(
                    root,
                    diagnostic_log,
                    options.query,
                    options.errors_on_failure,
                    &err,
                )?;
                return Err(err);
            }
        };
    let tree_context = CheckTreeContext {
        checked_tree_oid: options.tree_source.tree_oid_for_prompt_diff(root)?,
        against_tree_oid: options.against_tree.tree_oid_for_prompt_diff(root)?,
        against_tree: options.against_tree.clone(),
        checked_file_count: visible_tree_oid_cache.checked_file_count(root, options.tree_source)?,
    };
    let runner = LazyAppServerRunner::new(
        root,
        check_config_loads_plugins(config),
        &config.agent,
        options.no_sandbox,
    );
    Ok(PreparedCheckExecution {
        staged_view,
        tree_source: options.tree_source.clone(),
        tree_context,
        runner,
    })
}

fn write_prepare_check_failure(
    root: &Path,
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    errors_on_failure: usize,
    err: &str,
) -> Result<(), String> {
    write_check_error_finish_event(root, diagnostic_log, query, errors_on_failure, err)
}
