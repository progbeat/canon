use super::super::lifecycle::CheckCommandInspection;
use super::{run_prepared_check, CheckFailureOutput, PreparedCheckRun};
use crate::check::command::workflow::failure::CheckPublicOutputProgress;
use crate::check::command::{
    resolve_explicit_diff_from_tree_oids, run_with_token_usage_panic_capture, TokenUsageSummary,
};
use crate::check::core::{CheckCommandArgs, CheckRunReport};
use crate::check::interrogation::state::CheckRuntime;
use crate::check::ResolvedExpectation;
use crate::cli::CommandError;
use crate::logs::DiagnosticLogWriter;
use std::path::Path;
use std::time::Instant;

mod prepare;

use prepare::{prepare_git_backed_check, GitBackedPreparationContext, PreparedGitBackedCheck};

pub(in super::super) struct GitBackedCheckCommandContext<'a> {
    pub(in super::super) command_persistent_state_root:
        Option<&'a crate::state_paths::CanonStateRoot>,
    pub(in super::super) diagnostic_log: &'a mut DiagnosticLogWriter,
    pub(in super::super) started: Instant,
    pub(in super::super) public_output_progress: &'a mut CheckPublicOutputProgress,
    pub(in super::super) failure_output: &'a mut CheckFailureOutput,
    pub(in super::super) progress_report: &'a mut CheckRunReport,
    pub(in super::super) panic_token_usage: &'a mut TokenUsageSummary,
    pub(in super::super) inspection: &'a mut CheckCommandInspection,
}

pub(in super::super) fn run_git_backed_check_command(
    root: &Path,
    command: &CheckCommandArgs,
    context: GitBackedCheckCommandContext<'_>,
) -> Result<(), CommandError> {
    let GitBackedCheckCommandContext {
        command_persistent_state_root,
        diagnostic_log,
        started,
        public_output_progress,
        failure_output,
        progress_report,
        panic_token_usage,
        inspection,
    } = context;
    let PreparedGitBackedCheck {
        config,
        selection,
        mut check_caches,
        mut execution,
        feedback_context,
    } = prepare_git_backed_check(
        root,
        command,
        GitBackedPreparationContext {
            command_persistent_state_root,
            diagnostic_log,
            public_output_progress,
            failure_output,
            progress_report,
            inspection,
        },
    )?;
    let runtime = CheckRuntime::materialized(
        root,
        &execution.tree_materializer,
        &execution.tree_source,
        execution.tree_context.clone(),
        &config,
        command.no_sandbox,
    )
    .with_expectation_identities(&selection.identities);
    // [Tv] Cache filtering begins the engine's final preparation phase and
    // establishes the final Selected set. The next step invokes this callback
    // once to resolve that set's symbolic diff trees before evaluation begins.
    let mut resolve_selected_diff_from_tree_oids =
        |selected: &[ResolvedExpectation],
         repo_inspection: &mut crate::repo_inspection::RepoInspectionCache| {
            resolve_explicit_diff_from_tree_oids(
                root,
                selected
                    .iter()
                    .map(|expectation| expectation.diff_from.as_str()),
                repo_inspection,
                &execution.resources,
            )
        };
    failure_output.mark_ready_for_evaluation();
    run_with_token_usage_panic_capture(&mut execution.runner, panic_token_usage, |runner| {
        run_prepared_check(PreparedCheckRun {
            runtime,
            options: &selection.options,
            runner,
            check_caches: &mut check_caches,
            diagnostic_log,
            started,
            public_output_progress,
            progress_report,
            feedback_context,
            resolve_selected_diff_from_tree_oids: Some(&mut resolve_selected_diff_from_tree_oids),
        })
    })
}
