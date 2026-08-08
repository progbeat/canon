use super::AskQueryError;
use crate::cli::{CommandError, ReportedCommandFailure};

pub(super) fn ask_query_command_result(
    result: Result<(), AskQueryError>,
) -> Result<(), CommandError> {
    match result {
        Ok(()) => Ok(()),
        // [2Z] Reporting provenance, rather than the technical cause, decides
        // whether the CLI must print a diagnostic. This keeps setup and output
        // failures visible while avoiding a duplicate for a finished query.
        Err(AskQueryError::Unreported(message)) => Err(CommandError::from(message)),
        Err(AskQueryError::Reported(_)) => Err(CommandError::Reported(ReportedCommandFailure::Ask)),
    }
}
