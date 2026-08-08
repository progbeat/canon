pub(crate) fn install_check_signal_handlers() -> Result<(), String> {
    Ok(())
}

pub(crate) struct CheckTerminalEchoGuard;

pub(crate) fn interactive_check_terminal() -> bool {
    false
}

pub(crate) fn suppress_interactive_check_terminal_echo() -> Result<CheckTerminalEchoGuard, String> {
    Ok(CheckTerminalEchoGuard)
}

impl CheckTerminalEchoGuard {
    pub(crate) fn restore(self) -> Result<(), String> {
        Ok(())
    }
}
