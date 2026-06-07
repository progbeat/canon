use super::error::{PlatformError, PlatformResult};
use crate::platform::CHECK_INTERRUPTED;
use std::io;
use std::mem;
use std::sync::atomic::Ordering;

extern "C" fn handle_check_signal(_: i32) {
    CHECK_INTERRUPTED.store(true, Ordering::SeqCst);
}

pub(crate) fn install_check_signal_handlers() -> PlatformResult<()> {
    for signal_number in [libc::SIGHUP, libc::SIGINT, libc::SIGTERM] {
        install_signal_handler(signal_number)?;
    }
    Ok(())
}

fn install_signal_handler(signal_number: libc::c_int) -> PlatformResult<()> {
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
        Err(PlatformError::io(
            format!(
                "failed to install signal handler for signal {}",
                signal_number
            ),
            io::Error::last_os_error(),
        ))
    } else {
        Ok(())
    }
}
