use crate::app::LazyAppServerRunner;
use crate::check::command::workflow::failure::CheckPublicOutputProgress;
use crate::check::command::workflow::failure::{
    or_fail_at_selection_boundary, CheckFailureOutput, SelectionBoundary,
};
use crate::check::config::validation::check_config_loads_plugins;
use crate::check::core::{CheckCommandArgs, CheckRunReport};
use crate::check::CheckRunCaches;
use crate::cli::CommandError;
use crate::config_types::CheckConfig;
use crate::evaluator::EvaluatorProcessIsolation;
use crate::logs::DiagnosticLogWriter;
use crate::state_paths::CanonStateRoot;
use std::path::Path;

mod configuration;

use configuration::{
    prepare_in_place_configuration, InPlaceConfigurationContext, PreparedInPlaceConfiguration,
};

pub(super) struct InPlacePreparationContext<'a> {
    pub(super) command_persistent_state_root: Option<&'a CanonStateRoot>,
    pub(super) diagnostic_log: &'a mut DiagnosticLogWriter,
    pub(super) public_output_progress: &'a mut CheckPublicOutputProgress,
    pub(super) failure_output: &'a mut CheckFailureOutput,
    pub(super) progress_report: &'a mut CheckRunReport,
}

pub(super) struct PreparedInPlaceCheck {
    pub(super) config: CheckConfig,
    pub(super) selection: super::super::selection::CheckSelection,
    pub(super) check_caches: CheckRunCaches,
    pub(super) runner: LazyAppServerRunner,
    pub(super) persistent_status_history: bool,
}

pub(super) fn prepare_in_place_check(
    root: &Path,
    command: &CheckCommandArgs,
    context: InPlacePreparationContext<'_>,
) -> Result<PreparedInPlaceCheck, CommandError> {
    let InPlacePreparationContext {
        command_persistent_state_root,
        diagnostic_log,
        public_output_progress,
        failure_output,
        progress_report,
    } = context;
    let PreparedInPlaceConfiguration {
        config,
        selection,
        check_caches,
        persistent_status_history,
    } = prepare_in_place_configuration(
        root,
        command,
        InPlaceConfigurationContext {
            command_persistent_state_root,
            diagnostic_log,
            public_output_progress,
            failure_output,
            progress_report,
        },
    )?;
    let process_isolation = if command.no_sandbox {
        EvaluatorProcessIsolation::ExternallyManaged
    } else {
        EvaluatorProcessIsolation::CanonManaged
    };
    let runner = or_fail_at_selection_boundary(
        LazyAppServerRunner::new_in_place(
            check_config_loads_plugins(&config),
            &config.agent,
            process_isolation,
        ),
        SelectionBoundary::After,
        diagnostic_log,
        public_output_progress,
        failure_output,
    )?;
    Ok(PreparedInPlaceCheck {
        config,
        selection,
        check_caches,
        runner,
        persistent_status_history,
    })
}
