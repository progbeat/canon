use super::error::{PlatformError, PlatformResult};
use crate::platform::wait_for_app_server_child;
use std::io;
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) fn prepare_app_server_command(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

pub(crate) fn terminate_app_server_child(child: &mut Child) -> PlatformResult<()> {
    if poll_app_server_child(child)?.is_some() {
        return Ok(());
    }
    let process_group = app_server_process_group(child)?;
    let mut errors = Vec::new();
    signal_process_group_or_kill_child(child, process_group, libc::SIGTERM, &mut errors);
    if wait_for_child_exit(child, Duration::from_secs(2))? {
        return finish_app_server_cleanup(errors);
    }
    signal_process_group_or_kill_child(child, process_group, libc::SIGKILL, &mut errors);
    if let Err(err) = wait_for_app_server_child(child) {
        errors.push(PlatformError::message(err));
    }
    finish_app_server_cleanup(errors)
}

fn app_server_process_group(child: &Child) -> PlatformResult<libc::pid_t> {
    let pid = child.id();
    libc::pid_t::try_from(pid).map_err(|_| {
        PlatformError::message(format!(
            "app-server child pid {} does not fit Unix process group id",
            pid
        ))
    })
}

fn signal_process_group(
    process_group: libc::pid_t,
    signal_number: libc::c_int,
) -> PlatformResult<()> {
    // SAFETY: POSIX `kill` uses a negative pid to address a process group.
    // The app-server child is spawned in its own process group first.
    let result = unsafe { libc::kill(-process_group, signal_number) };
    if result == 0 {
        Ok(())
    } else {
        Err(PlatformError::io(
            format!(
                "failed to send signal {} to app-server process group {}",
                signal_number, process_group
            ),
            io::Error::last_os_error(),
        ))
    }
}

fn signal_process_group_or_kill_child(
    child: &mut Child,
    process_group: libc::pid_t,
    signal_number: libc::c_int,
    errors: &mut Vec<PlatformError>,
) {
    if let Err(err) = signal_process_group(process_group, signal_number) {
        if child_already_exited(child, errors) {
            return;
        }
        errors.push(err);
        if let Err(err) = child.kill() {
            if !child_already_exited(child, errors) {
                errors.push(PlatformError::io("failed to kill app-server child", err));
            }
        }
    }
}

fn child_already_exited(child: &mut Child, errors: &mut Vec<PlatformError>) -> bool {
    match child.try_wait() {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(err) => {
            errors.push(PlatformError::io("failed to poll app-server child", err));
            false
        }
    }
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> PlatformResult<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        if poll_app_server_child(child)?.is_some() {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn poll_app_server_child(child: &mut Child) -> PlatformResult<Option<ExitStatus>> {
    child
        .try_wait()
        .map_err(|err| PlatformError::io("failed to poll app-server child", err))
}

fn finish_app_server_cleanup(errors: Vec<PlatformError>) -> PlatformResult<()> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(PlatformError::chain(errors))
    }
}
