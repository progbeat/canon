use super::super::super::lifecycle::CheckCommandInspection;
use super::super::or_fail_with_default_output;
use crate::check::command::output::CheckFeedbackContext;
use crate::check::command::workflow::failure::CheckFailureOutput;
use crate::check::command::workflow::failure::CheckPublicOutputProgress;
use crate::check::command::workflow::prepare::PreparedGitBackedCheckExecution;
use crate::check::command::workflow::trailer::check_command_emits_feedback;
use crate::check::core::{CheckCommandArgs, CheckRunReport};
use crate::check::CheckRunCaches;
use crate::cli::CommandError;
use crate::logs::DiagnosticLogWriter;
use crate::state_paths::CanonStateRoot;
use std::path::Path;

mod configuration;
mod execution;
mod trees;

use configuration::{
    prepare_git_backed_configuration, GitBackedConfigurationContext, PreparedGitBackedConfiguration,
};
use execution::{prepare_git_backed_execution, GitBackedExecutionContext};
use trees::{resolve_git_backed_check_trees, ResolvedGitBackedCheckTrees};

pub(super) struct GitBackedPreparationContext<'a> {
    pub(super) command_persistent_state_root: Option<&'a CanonStateRoot>,
    pub(super) diagnostic_log: &'a mut DiagnosticLogWriter,
    pub(super) public_output_progress: &'a mut CheckPublicOutputProgress,
    pub(super) failure_output: &'a mut CheckFailureOutput,
    pub(super) progress_report: &'a mut CheckRunReport,
    pub(super) inspection: &'a mut CheckCommandInspection,
}

pub(super) struct PreparedGitBackedCheck {
    pub(super) config: crate::config_types::CheckConfig,
    pub(super) selection: super::super::selection::CheckSelection,
    pub(super) check_caches: CheckRunCaches,
    pub(super) execution: PreparedGitBackedCheckExecution,
    pub(super) feedback_context: Option<CheckFeedbackContext>,
}

pub(super) fn prepare_git_backed_check(
    root: &Path,
    command: &CheckCommandArgs,
    context: GitBackedPreparationContext<'_>,
) -> Result<PreparedGitBackedCheck, CommandError> {
    let GitBackedPreparationContext {
        command_persistent_state_root,
        diagnostic_log,
        public_output_progress,
        failure_output,
        progress_report,
        inspection,
    } = context;
    let emit_feedback = check_command_emits_feedback(command);
    let mut check_caches =
        CheckRunCaches::with_repo_inspection_cache(inspection.repo_inspection.clone());
    if let Some(command_persistent_state_root) = command_persistent_state_root {
        check_caches
            .xpec_state
            .bind_state_root(root, command_persistent_state_root);
    }
    let resources = inspection.git_resources.share_persistent();
    let ResolvedGitBackedCheckTrees {
        checked_tree,
        tree_context,
    } = or_fail_with_default_output(
        resolve_git_backed_check_trees(root, command, &mut check_caches, &resources),
        root,
        false,
        diagnostic_log,
        public_output_progress,
        failure_output,
        inspection,
    )?;
    let feedback_context = if emit_feedback {
        let head_tree_oid = tree_context
            .head_tree_oid
            .as_deref()
            .expect("persistent Git-backed check preparation resolves HEAD");
        Some(CheckFeedbackContext::from_tree_oids(
            &tree_context.checked_tree_oid,
            &tree_context.against_tree_oid,
            head_tree_oid,
        ))
    } else {
        None
    };
    if let Some(feedback_context) = feedback_context {
        *failure_output = failure_output.with_feedback_context(feedback_context);
    }
    let PreparedGitBackedConfiguration { config, selection } = prepare_git_backed_configuration(
        root,
        command,
        &checked_tree,
        &mut check_caches,
        GitBackedConfigurationContext {
            diagnostic_log,
            public_output_progress,
            failure_output,
            progress_report,
            persistent_state: command_persistent_state_root.is_some(),
        },
    )?;
    let execution = prepare_git_backed_execution(
        root,
        &config,
        &checked_tree,
        tree_context,
        command.no_sandbox,
        resources,
        &check_caches,
        GitBackedExecutionContext {
            diagnostic_log,
            public_output_progress,
            failure_output,
        },
    )?;
    Ok(PreparedGitBackedCheck {
        config,
        selection,
        check_caches,
        execution,
        feedback_context,
    })
}
