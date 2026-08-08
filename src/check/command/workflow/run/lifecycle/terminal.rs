use crate::cli::CommandError;

pub(super) fn with_suppressed_terminal_echo(
    run: impl FnOnce() -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    // [sj] This lifecycle boundary is the implementation of canon_check's
    // command-wide `@echo_off`: it wraps preparation, evaluation, and trailers.
    let terminal_echo = crate::platform::process::suppress_interactive_check_terminal_echo()
        .map_err(CommandError::from)?;
    let result = run();
    let restore = terminal_echo.restore().map_err(CommandError::from);
    match (result, restore) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(primary), Err(restore)) => Err(format!("{primary}; also {restore}").into()),
    }
}
