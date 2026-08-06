use crate::cli::CommandError;
use crate::logs::{DiagnosticLogPlan, DiagnosticLogResult, DiagnosticLogWriter};
use crate::state_paths::CanonStateRoot;
use std::path::Path;

pub(super) fn start_check_diagnostic_log(
    root: &Path,
    command_persistent_state_root: Option<&CanonStateRoot>,
    git_backed_plan: Option<DiagnosticLogPlan>,
) -> DiagnosticLogResult<DiagnosticLogWriter> {
    // [w,90,g2] Runtime-event ownership begins before fallible parsing
    // and tree preparation. The already prepared command control-plane value
    // is separate from either mode's checked subject. In-place has no such
    // plan because its canonical contract ignores Git configuration.
    let mut diagnostic_log = match git_backed_plan {
        Some(plan) => DiagnosticLogWriter::create_from_plan(root, plan),
        None => DiagnosticLogWriter::create_in_place(command_persistent_state_root),
    }?;
    // [w] Runtime observability must not interrupt preparation, evaluation, or
    // public finally effects. Every event write remains unconditional; the
    // writer returns its first storage failure after the whole command lifecycle.
    diagnostic_log.defer_write_errors();
    Ok(diagnostic_log)
}

pub(super) fn finish_check_command(
    result: Result<(), CommandError>,
    diagnostic_log_error: Option<String>,
) -> Result<(), CommandError> {
    match diagnostic_log_error {
        Some(error) => match result {
            Ok(()) => Err(format!("failed to write check runtime log: {error}").into()),
            Err(primary) => {
                Err(format!("{primary}; also failed to write check runtime log: {error}").into())
            }
        },
        None => result,
    }
}

#[cfg(test)]
mod tests {
    use super::finish_check_command;
    use crate::cli::{CommandError, ReportedCommandFailure};

    #[test] // xpec: w
    fn deferred_check_log_error_is_returned_after_primary_result() {
        let error = finish_check_command(
            Err(CommandError::Reported(ReportedCommandFailure::Check)),
            Some("sink failed".to_string()),
        )
        .unwrap_err();

        assert_eq!(
            error,
            CommandError::from(
                "canon check failed; also failed to write check runtime log: sink failed"
                    .to_string()
            )
        );
    }
}
