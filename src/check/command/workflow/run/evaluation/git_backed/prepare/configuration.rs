use super::super::super::selection::{
    record_collected_expectations, resolve_check_selection,
    retain_only_current_configuration_xpec_state, start_check_with_candidates, CheckSelection,
};
use crate::check::command::workflow::failure::CheckPublicOutputProgress;
use crate::check::command::workflow::failure::{
    or_fail_at_selection_boundary, CheckFailureOutput, SelectionBoundary,
};
use crate::check::config::collect_check_config;
use crate::check::core::{CheckCommandArgs, CheckRunReport};
use crate::check::CheckRunCaches;
use crate::cli::CommandError;
use crate::config_types::CheckConfig;
use crate::git::TreeSource;
use crate::logs::DiagnosticLogWriter;
use std::path::Path;

pub(super) struct GitBackedConfigurationContext<'a> {
    pub(super) diagnostic_log: &'a mut DiagnosticLogWriter,
    pub(super) public_output_progress: &'a mut CheckPublicOutputProgress,
    pub(super) failure_output: &'a mut CheckFailureOutput,
    pub(super) progress_report: &'a mut CheckRunReport,
    pub(super) persistent_state: bool,
}

pub(super) struct PreparedGitBackedConfiguration {
    pub(super) config: CheckConfig,
    pub(super) selection: CheckSelection,
}

pub(super) fn prepare_git_backed_configuration(
    root: &Path,
    command: &CheckCommandArgs,
    checked_tree: &TreeSource,
    check_caches: &mut CheckRunCaches,
    context: GitBackedConfigurationContext<'_>,
) -> Result<PreparedGitBackedConfiguration, CommandError> {
    let GitBackedConfigurationContext {
        diagnostic_log,
        public_output_progress,
        failure_output,
        progress_report,
        persistent_state,
    } = context;
    // [w] If config collection fails, the required summary reports the empty
    // collected outcome domain and feedback requires the reported error to be
    // fixed. This state must be recorded before the failure boundary writes
    // the trailer.
    let collected_config_result = collect_check_config(
        &mut check_caches.repo_inspection,
        root,
        &command.config_path,
        checked_tree,
    );
    if collected_config_result.is_err() {
        failure_output.mark_collection_failed();
    }
    let collected_config = or_fail_at_selection_boundary(
        collected_config_result,
        SelectionBoundary::Before,
        diagnostic_log,
        public_output_progress,
        failure_output,
    )?;
    record_collected_expectations(
        collected_config.expectation_count(),
        failure_output,
        progress_report,
    );
    let config = or_fail_at_selection_boundary(
        collected_config.into_validated(),
        SelectionBoundary::Before,
        diagnostic_log,
        public_output_progress,
        failure_output,
    )?;
    let selection = or_fail_at_selection_boundary(
        resolve_check_selection(&config, &command.options),
        SelectionBoundary::Before,
        diagnostic_log,
        public_output_progress,
        failure_output,
    )?;
    start_check_with_candidates(
        &selection,
        diagnostic_log,
        public_output_progress,
        failure_output,
    )?;
    if persistent_state {
        // [fh] Normal Git-backed checks exhaustively remove per-ID state that
        // is absent from the complete current configuration before evaluation.
        retain_only_current_configuration_xpec_state(
            &mut check_caches.xpec_state,
            root,
            &selection.identities,
            diagnostic_log,
            public_output_progress,
            failure_output,
        )?;
    }
    Ok(PreparedGitBackedConfiguration { config, selection })
}
