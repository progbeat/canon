use crate::project::command_output_trimmed;
use std::process::Command;

pub(crate) fn run_git_command(
    command: &mut Command,
    run_description: &str,
    failure_description: &str,
) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|err| format!("failed to run {}: {}", run_description, err))?;
    if !output.status.success() {
        return Err(format!(
            "{}: {}",
            failure_description,
            command_output_trimmed(&output.stderr, "git stderr")?
        ));
    }
    Ok(())
}
