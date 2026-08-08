use super::super::evaluation::or_fail_with_default_output;
use super::CheckCommandInspection;
use crate::check::cli_args::args_request_in_place;
use crate::check::command::workflow::failure::CheckFailureOutput;
use crate::check::command::workflow::failure::CheckPublicOutputProgress;
use crate::check::core::CheckCommandArgs;
use crate::cli::CommandError;
use crate::logs::DiagnosticLogWriter;
use crate::platform::process::{install_check_signal_handlers, reset_check_interrupted};
use std::ffi::OsString;
use std::path::Path;

pub(super) fn prepare_check_command(
    root: &Path,
    parsed_command: Result<CheckCommandArgs, String>,
    in_place: bool,
    diagnostic_log: &mut DiagnosticLogWriter,
    public_output_progress: &mut CheckPublicOutputProgress,
    failure_output: &mut CheckFailureOutput,
    inspection: &mut CheckCommandInspection,
) -> Result<CheckCommandArgs, CommandError> {
    or_fail_with_default_output(
        install_check_signal_handlers(),
        root,
        in_place,
        diagnostic_log,
        public_output_progress,
        failure_output,
        inspection,
    )?;
    reset_check_interrupted();
    or_fail_with_default_output(
        parsed_command,
        root,
        in_place,
        diagnostic_log,
        public_output_progress,
        failure_output,
        inspection,
    )
}

pub(super) fn preparse_args_use_in_place(args: &[OsString], default_in_place: bool) -> bool {
    default_in_place || args_request_in_place(args)
}
