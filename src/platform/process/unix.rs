use super::CHECK_INTERRUPTED;
use std::io;
use std::io::IsTerminal;
use std::mem;
use std::sync::atomic::Ordering;

extern "C" fn handle_check_signal(_: i32) {
    CHECK_INTERRUPTED.store(true, Ordering::SeqCst);
}

pub(crate) fn install_check_signal_handlers() -> Result<(), String> {
    for signal_number in [libc::SIGHUP, libc::SIGINT, libc::SIGTERM] {
        install_signal_handler(signal_number)?;
    }
    Ok(())
}

fn install_signal_handler(signal_number: libc::c_int) -> Result<(), String> {
    // SAFETY: `sigaction` is initialized before use, the handler has C ABI and
    // only stores to an atomic flag, and libc reports failure via the return
    // value checked immediately below.
    let result = unsafe {
        let mut action: libc::sigaction = mem::zeroed();
        action.sa_flags = 0;
        action.sa_sigaction = handle_check_signal as *const () as usize;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(signal_number, &action, std::ptr::null_mut())
    };
    if result == -1 {
        Err(format!(
            "failed to install signal handler for signal {}: {}",
            signal_number,
            io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

pub(crate) struct CheckTerminalEchoGuard {
    original: Option<libc::termios>,
}

pub(crate) fn interactive_check_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

pub(crate) fn suppress_interactive_check_terminal_echo() -> Result<CheckTerminalEchoGuard, String> {
    if !interactive_check_terminal() {
        return Ok(CheckTerminalEchoGuard { original: None });
    }
    // [sj] POSIX half of `echo_off`; the guard's explicit restore plus Drop
    // preserve the original attributes on every command exit path.
    let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: `original` points to writable storage for one termios value and
    // STDIN_FILENO is the terminal descriptor proven above.
    if unsafe { libc::tcgetattr(libc::STDIN_FILENO, original.as_mut_ptr()) } != 0 {
        return Err(terminal_error("failed to read terminal attributes"));
    }
    // SAFETY: tcgetattr initialized `original` after the successful return.
    let original = unsafe { original.assume_init() };
    let mut echo_off = original;
    echo_off.c_lflag &= !(libc::ECHO | libc::ECHONL);
    // SAFETY: `echo_off` is an initialized termios value for STDIN_FILENO.
    if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &echo_off) } != 0 {
        return Err(terminal_error("failed to suppress terminal echo"));
    }
    Ok(CheckTerminalEchoGuard {
        original: Some(original),
    })
}

impl CheckTerminalEchoGuard {
    pub(crate) fn restore(mut self) -> Result<(), String> {
        let result = self.restore_inner();
        self.original = None;
        result
    }

    fn restore_inner(&self) -> Result<(), String> {
        let Some(original) = self.original.as_ref() else {
            return Ok(());
        };
        // SAFETY: `original` came from tcgetattr for STDIN_FILENO.
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, original) } != 0 {
            return Err(terminal_error("failed to restore terminal attributes"));
        }
        Ok(())
    }
}

fn terminal_error(context: &str) -> String {
    format!("{context}: {}", io::Error::last_os_error())
}

impl Drop for CheckTerminalEchoGuard {
    fn drop(&mut self) {
        let _ = self.restore_inner();
    }
}
