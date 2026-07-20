use super::wait_for_app_server_child;
use std::process::{Child, Command};

pub(crate) fn prepare_app_server_command(_command: &mut Command) {}

pub(crate) fn terminate_app_server_child(child: &mut Child) -> Result<(), String> {
    let mut errors = Vec::new();
    if let Err(err) = child.kill() {
        errors.push(format!("failed to kill app-server child: {}", err));
    }
    if let Err(err) = wait_for_app_server_child(child) {
        errors.push(err);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}
