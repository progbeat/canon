use super::super::super::selection::{
    record_collected_expectations, resolve_check_selection,
    retain_only_current_configuration_xpec_state, start_check_with_candidates, CheckSelection,
};
use crate::check::command::workflow::failure::CheckPublicOutputProgress;
use crate::check::command::workflow::failure::{
    or_fail_at_selection_boundary, CheckFailureOutput, SelectionBoundary,
};
use crate::check::config::collect_in_place_check_config_with_default_agent_preset;
use crate::check::core::{CheckCommandArgs, CheckRunReport};
use crate::check::CheckRunCaches;
use crate::cli::CommandError;
use crate::config_types::CheckConfig;
use crate::logs::DiagnosticLogWriter;
use crate::state_paths::CanonStateRoot;
use std::path::Path;

pub(super) struct InPlaceConfigurationContext<'a> {
    pub(super) command_persistent_state_root: Option<&'a CanonStateRoot>,
    pub(super) diagnostic_log: &'a mut DiagnosticLogWriter,
    pub(super) public_output_progress: &'a mut CheckPublicOutputProgress,
    pub(super) failure_output: &'a mut CheckFailureOutput,
    pub(super) progress_report: &'a mut CheckRunReport,
}

pub(super) struct PreparedInPlaceConfiguration {
    pub(super) config: CheckConfig,
    pub(super) selection: CheckSelection,
    pub(super) check_caches: CheckRunCaches,
    pub(super) persistent_status_history: bool,
}

pub(super) fn prepare_in_place_configuration(
    root: &Path,
    command: &CheckCommandArgs,
    context: InPlaceConfigurationContext<'_>,
) -> Result<PreparedInPlaceConfiguration, CommandError> {
    let InPlaceConfigurationContext {
        command_persistent_state_root,
        diagnostic_log,
        public_output_progress,
        failure_output,
        progress_report,
    } = context;
    // [g2,90] In-place invocation-local caches stay in this fresh in-memory
    // bundle. Status-specific last results are separate cross-invocation xpec
    // history and contain no Git-tree fields.
    let mut check_caches = CheckRunCaches::new();
    let collected_config = or_fail_at_selection_boundary(
        collect_in_place_check_config_with_default_agent_preset(
            &mut check_caches.repo_inspection,
            root,
            &command.config_path,
            None,
        ),
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
    let validated_config_and_selection = collected_config.into_validated().and_then(|config| {
        let selection = resolve_check_selection(config.config(), &command.options)?;
        Ok((config, selection))
    });
    let (in_place_config, selection) = or_fail_at_selection_boundary(
        validated_config_and_selection,
        SelectionBoundary::Before,
        diagnostic_log,
        public_output_progress,
        failure_output,
    )?;
    or_fail_at_selection_boundary(
        in_place_config.validate_configured_fields(),
        SelectionBoundary::Before,
        diagnostic_log,
        public_output_progress,
        failure_output,
    )?;
    if let Some(command_persistent_state_root) = command_persistent_state_root {
        // [1g,90] In-place status history uses the resolved canon-owned output
        // namespace as opaque control-plane storage, never as Git evaluation
        // input.
        check_caches
            .xpec_state
            .bind_in_place_state_root(root, command_persistent_state_root);
    }
    start_check_with_candidates(
        &selection,
        diagnostic_log,
        public_output_progress,
        failure_output,
    )?;
    if command_persistent_state_root.is_some() {
        // [fh] Retention sees the complete collected identity set before any
        // evaluation can write status history.
        retain_only_current_configuration_xpec_state(
            &mut check_caches.xpec_state,
            root,
            &selection.identities,
            diagnostic_log,
            public_output_progress,
            failure_output,
        )?;
    }
    Ok(PreparedInPlaceConfiguration {
        config: in_place_config.into_config(),
        selection,
        check_caches,
        persistent_status_history: command_persistent_state_root.is_some(),
    })
}
