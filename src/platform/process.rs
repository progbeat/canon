use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

static CHECK_INTERRUPTED: AtomicBool = AtomicBool::new(false);

pub(crate) fn install_check_signal_handlers() -> Result<(), String> {
    imp::install_check_signal_handlers()
}

pub(crate) fn reset_check_interrupted() {
    CHECK_INTERRUPTED.store(false, Ordering::SeqCst);
}

pub(crate) fn check_interrupted() -> bool {
    CHECK_INTERRUPTED.load(Ordering::SeqCst)
}

pub(crate) struct CheckTerminalEchoGuard {
    inner: imp::CheckTerminalEchoGuard,
}

impl CheckTerminalEchoGuard {
    pub(crate) fn restore(self) -> Result<(), String> {
        self.inner.restore()
    }
}

pub(crate) fn suppress_interactive_check_terminal_echo() -> Result<CheckTerminalEchoGuard, String> {
    imp::suppress_interactive_check_terminal_echo().map(|inner| CheckTerminalEchoGuard { inner })
}

pub(crate) fn interactive_check_terminal() -> bool {
    imp::interactive_check_terminal()
}
