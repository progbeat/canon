use super::wait_for_app_server_child;
use std::process::{Child, Command};

pub(crate) fn prepare_app_server_command(_command: &mut Command) {}

pub(crate) fn terminate_app_server_child(child: &mut Child) -> Result<(), String> {
    child
        .kill()
        .map_err(|err| format!("failed to kill app-server child: {}", err))?;
    wait_for_app_server_child(child)?;
    Ok(())
}
