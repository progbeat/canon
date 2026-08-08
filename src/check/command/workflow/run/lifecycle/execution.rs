use super::super::evaluation::prepare_default_failure_output;
use super::dispatch::dispatch_check_command;
use super::preflight::prepare_check_command;
use super::CheckCommandInspection;
use crate::check::command::workflow::failure::CheckPublicOutputProgress;
use crate::check::command::workflow::failure::{
    write_unconditional_check_trailer_and_feedback, CheckFailureOutput,
};
use crate::check::command::TokenUsageSummary;
use crate::check::core::{CheckCommandArgs, CheckRunReport};
use crate::cli::CommandError;
use crate::logs::DiagnosticLogWriter;
use std::path::Path;
use std::time::Instant;

#[allow(clippy::too_many_arguments)]
pub(super) fn run_check_command_with_writer(
    root: &Path,
    parsed_command: Result<CheckCommandArgs, String>,
    in_place: bool,
    command_persistent_state_root: Option<&crate::state_paths::CanonStateRoot>,
    started: Instant,
    public_output_progress: &mut CheckPublicOutputProgress,
    failure_output: &mut CheckFailureOutput,
    progress_report: &mut CheckRunReport,
    panic_token_usage: &mut TokenUsageSummary,
    diagnostic_log: &mut DiagnosticLogWriter,
    inspection: &mut CheckCommandInspection,
) -> Result<(), CommandError> {
    let command = prepare_check_command(
        root,
        parsed_command,
        in_place,
        diagnostic_log,
        public_output_progress,
        failure_output,
        inspection,
    )?;
    let result = dispatch_check_command(
        root,
        &command,
        command_persistent_state_root,
        diagnostic_log,
        started,
        public_output_progress,
        failure_output,
        progress_report,
        panic_token_usage,
        inspection,
    );
    if result.is_err() && public_output_progress.needs_trailer() {
        // [kK] This outer boundary is the `finally` path for failures that
        // occur before config selection or diagnostic logging can own the
        // token/summary/feedback trailer.
        public_output_progress.mark_all_attempted();
        write_default_failure_output(root, command.in_place, failure_output, inspection)?;
    }
    result
}

pub(super) fn write_default_failure_output(
    root: &Path,
    in_place: bool,
    output: &mut CheckFailureOutput,
    inspection: &mut CheckCommandInspection,
) -> Result<(), CommandError> {
    *output = prepare_default_failure_output(root, *output, in_place, inspection);
    write_unconditional_check_trailer_and_feedback(*output)
}
