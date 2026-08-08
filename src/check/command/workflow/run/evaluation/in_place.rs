use super::{run_prepared_check, CheckFailureOutput, PreparedCheckRun};
use crate::check::command::workflow::failure::CheckPublicOutputProgress;
use crate::check::command::{run_with_token_usage_panic_capture, TokenUsageSummary};
use crate::check::core::{CheckCommandArgs, CheckRunReport};
use crate::check::interrogation::state::CheckRuntime;
use crate::cli::CommandError;
use crate::logs::DiagnosticLogWriter;
use std::path::Path;
use std::time::Instant;

mod prepare;

use prepare::{prepare_in_place_check, InPlacePreparationContext, PreparedInPlaceCheck};

pub(in super::super) struct InPlaceCheckCommandContext<'a> {
    pub(in super::super) command_persistent_state_root:
        Option<&'a crate::state_paths::CanonStateRoot>,
    pub(in super::super) diagnostic_log: &'a mut DiagnosticLogWriter,
    pub(in super::super) started: Instant,
    pub(in super::super) public_output_progress: &'a mut CheckPublicOutputProgress,
    pub(in super::super) failure_output: &'a mut CheckFailureOutput,
    pub(in super::super) progress_report: &'a mut CheckRunReport,
    pub(in super::super) panic_token_usage: &'a mut TokenUsageSummary,
}

pub(in super::super) fn run_in_place_check_command(
    root: &Path,
    command: &CheckCommandArgs,
    context: InPlaceCheckCommandContext<'_>,
) -> Result<(), CommandError> {
    let InPlaceCheckCommandContext {
        command_persistent_state_root,
        diagnostic_log,
        started,
        public_output_progress,
        failure_output,
        progress_report,
        panic_token_usage,
    } = context;
    let PreparedInPlaceCheck {
        config,
        selection,
        mut check_caches,
        mut runner,
        persistent_status_history,
    } = prepare_in_place_check(
        root,
        command,
        InPlacePreparationContext {
            command_persistent_state_root,
            diagnostic_log,
            public_output_progress,
            failure_output,
            progress_report,
        },
    )?;
    let runtime = CheckRuntime::in_place(root, &config, persistent_status_history)
        .with_expectation_identities(&selection.identities);
    // The in-place runtime makes `run_check_with_runner_and_caches` build a
    // direct Evaluate-only work queue: no Git-backed cached evaluation is
    // selected. The common rank/latest-fail ordering still applies, completed
    // records are returned in this invocation's in-memory CheckRunReport, and a
    // canonical persistent namespace, when available, receives separate
    // status-specific xpec history without Git-tree fields.
    failure_output.mark_ready_for_evaluation();
    run_with_token_usage_panic_capture(&mut runner, panic_token_usage, |runner| {
        run_prepared_check(PreparedCheckRun {
            runtime,
            options: &selection.options,
            runner,
            check_caches: &mut check_caches,
            diagnostic_log,
            started,
            public_output_progress,
            progress_report,
            feedback_context: None,
            resolve_selected_diff_from_tree_oids: None,
        })
    })
}
