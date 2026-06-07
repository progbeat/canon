mod codex_home;
mod environment;
mod evaluator;
mod io;
mod readers;
mod runner;
mod spawn;
mod transport;
#[cfg(unix)]
mod unix;
mod usage;
#[cfg(windows)]
mod windows;

use std::process::{Child, Command};

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

pub(crate) use codex_home::prepare_evaluator_codex_home;
pub(crate) use environment::configure_app_server_environment;
pub(crate) use readers::{spawn_app_server_reader, spawn_app_server_stderr_reader};
pub(crate) use runner::AppServerRunner;
pub(crate) use transport::AppServerTurnRequest;

pub(crate) fn prepare_app_server_command(command: &mut Command) {
    imp::prepare_app_server_command(command);
}

pub(crate) fn terminate_app_server_child(child: &mut Child) -> Result<(), String> {
    imp::terminate_app_server_child(child)
}

fn wait_for_app_server_child(child: &mut Child) -> Result<(), String> {
    child
        .wait()
        .map(|_| ())
        .map_err(|err| format!("failed to wait for app-server child: {}", err))
}
