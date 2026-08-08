use super::super::evaluation::{
    run_git_backed_check_command, run_in_place_check_command, GitBackedCheckCommandContext,
    InPlaceCheckCommandContext,
};
use super::CheckCommandInspection;
use crate::check::command::workflow::failure::CheckFailureOutput;
use crate::check::command::workflow::failure::CheckPublicOutputProgress;
use crate::check::command::TokenUsageSummary;
use crate::check::core::{CheckCommandArgs, CheckRunReport};
use crate::cli::CommandError;
use crate::logs::DiagnosticLogWriter;
use crate::state_paths::CanonStateRoot;
use std::path::Path;
use std::time::Instant;

#[allow(clippy::too_many_arguments)]
pub(super) fn dispatch_check_command(
    root: &Path,
    command: &CheckCommandArgs,
    command_persistent_state_root: Option<&CanonStateRoot>,
    diagnostic_log: &mut DiagnosticLogWriter,
    started: Instant,
    public_output_progress: &mut CheckPublicOutputProgress,
    failure_output: &mut CheckFailureOutput,
    progress_report: &mut CheckRunReport,
    panic_token_usage: &mut TokenUsageSummary,
    inspection: &mut CheckCommandInspection,
) -> Result<(), CommandError> {
    if command.in_place {
        // In-place exits before the Git-backed path can inspect trees or
        // select cached evaluations. Its dedicated path reads checked contents
        // from the filesystem and separately maintains canon-owned last-result
        // history, bounded state retention, and invocation-local runtime logs.
        return run_in_place_check_command(
            root,
            command,
            InPlaceCheckCommandContext {
                command_persistent_state_root,
                diagnostic_log,
                started,
                public_output_progress,
                failure_output,
                progress_report,
                panic_token_usage,
            },
        );
    }
    run_git_backed_check_command(
        root,
        command,
        GitBackedCheckCommandContext {
            command_persistent_state_root,
            diagnostic_log,
            started,
            public_output_progress,
            failure_output,
            progress_report,
            panic_token_usage,
            inspection,
        },
    )
}
