//! Isolated child environment and operating-system process control.

mod codex_home;
mod environment;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use std::process::{Child, Command};

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

pub(super) use codex_home::{prepare_evaluator_codex_home, EvaluatorCodexHome};
pub(super) use environment::configure_app_server_environment;

pub(super) fn prepare_app_server_command(command: &mut Command) {
    imp::prepare_app_server_command(command);
}

pub(super) fn terminate_app_server_child(child: &mut Child) -> Result<(), String> {
    imp::terminate_app_server_child(child)
}

fn wait_for_app_server_child(child: &mut Child) -> Result<(), String> {
    child
        .wait()
        .map(|_| ())
        .map_err(|err| format!("failed to wait for app-server child: {}", err))
}
