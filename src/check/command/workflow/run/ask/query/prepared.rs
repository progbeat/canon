use super::AskQueryError;
use crate::app::LazyAppServerRunner;
use crate::check::command::{
    prepare_git_backed_check_execution, resolve_explicit_diff_from_tree_oids,
    GitBackedCheckResources, PrepareGitBackedCheckExecutionOptions, TokenUsageSummary,
};
use crate::check::interrogation::state::{CheckRuntime, CheckTreeContext};
use crate::check::CheckRunCaches;
use crate::config_types::CheckConfig;
use crate::git::TreeSource;
use crate::logs::DiagnosticLogWriter;
use std::path::Path;

mod evaluation;
mod output;

use evaluation::evaluate_prepared_ask_query;

pub(super) struct StartedAskQueryCommand<'a, 'b> {
    pub(super) root: &'a Path,
    pub(super) question: &'a str,
    pub(super) config: &'a CheckConfig,
    pub(super) tree_source: Option<TreeSource>,
    pub(super) tree_context: Option<CheckTreeContext>,
    pub(super) resources: Option<GitBackedCheckResources>,
    pub(super) in_place: bool,
    pub(super) no_sandbox: bool,
    pub(super) diagnostic_log: Option<&'b mut DiagnosticLogWriter>,
    pub(super) check_caches: &'a mut CheckRunCaches,
    pub(super) token_usage_summary: &'a mut TokenUsageSummary,
}

pub(super) fn run_started_ask_query_command(
    command: StartedAskQueryCommand<'_, '_>,
) -> Result<(), AskQueryError> {
    let StartedAskQueryCommand {
        root,
        question,
        config,
        tree_source,
        tree_context,
        resources,
        in_place,
        no_sandbox,
        mut diagnostic_log,
        check_caches,
        token_usage_summary,
    } = command;
    let mut in_place_runner;
    let (runtime, runner): (CheckRuntime<'_>, &mut LazyAppServerRunner) = if in_place {
        let process_isolation = if no_sandbox {
            crate::evaluator::EvaluatorProcessIsolation::ExternallyManaged
        } else {
            crate::evaluator::EvaluatorProcessIsolation::CanonManaged
        };
        // [l] Ask uses a state-free, read-only evaluator runner. Externally
        // managed process isolation delegates enforcement to the caller but does
        // not expose a write-capable project tool; runtime logs remain memory-only.
        in_place_runner = LazyAppServerRunner::new_in_place(
            crate::check::config::validation::check_config_loads_plugins(config),
            &config.agent,
            process_isolation,
        )?;
        (
            CheckRuntime::in_place_temporary_query(root, config),
            &mut in_place_runner,
        )
    } else {
        let tree_source = tree_source.ok_or_else(|| "missing query tree source".to_string())?;
        let mut tree_context =
            tree_context.ok_or_else(|| "missing query tree context".to_string())?;
        let resources = resources.ok_or_else(|| "missing query Git resources".to_string())?;
        // [l] Git-backed ask also keeps prompt objects and materialized trees
        // invocation-local, with no app-server or xpec history sink.
        tree_context.explicit_diff_from_tree_oids = resolve_explicit_diff_from_tree_oids(
            root,
            config
                .expectations
                .iter()
                .map(|expectation| expectation.diff_from.as_str()),
            &mut check_caches.repo_inspection,
            &resources,
        )?;
        let mut execution = prepare_git_backed_check_execution(
            root,
            config,
            PrepareGitBackedCheckExecutionOptions {
                tree_source: &tree_source,
                tree_context,
                no_sandbox,
                resources,
                repo_inspection: check_caches.repo_inspection.clone(),
                temporary_directory_allocator: &check_caches.temporary_directory_allocator,
            },
        )?;
        let runtime = CheckRuntime::materialized_without_persistent_history(
            root,
            &execution.tree_materializer,
            &execution.tree_source,
            execution.tree_context.clone(),
            config,
            false,
        );
        let runner = &mut execution.runner;
        return evaluate_prepared_ask_query(
            runtime,
            runner,
            question,
            config,
            &mut diagnostic_log,
            check_caches,
            token_usage_summary,
        );
    };
    evaluate_prepared_ask_query(
        runtime,
        runner,
        question,
        config,
        &mut diagnostic_log,
        check_caches,
        token_usage_summary,
    )
}
