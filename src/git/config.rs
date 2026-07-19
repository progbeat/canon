use crate::project::command_output_trimmed;
use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus};

#[derive(Debug)]
pub(crate) enum GitConfigGetError {
    Command(io::Error),
    InvalidOutput {
        stream: &'static str,
        message: String,
    },
    ReadFailed {
        status: String,
        stderr: String,
    },
}

pub(crate) fn git_config_get(root: &Path, key: &str) -> Result<Option<String>, GitConfigGetError> {
    let mut command = project_config_command(root);
    run_config_get(&mut command, key)
}

fn project_config_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).arg("config");
    command
}

fn run_config_get(command: &mut Command, key: &str) -> Result<Option<String>, GitConfigGetError> {
    let output = command
        .arg("--get")
        .arg(key)
        .output()
        .map_err(GitConfigGetError::Command)?;
    if output.status.success() {
        let stdout =
            command_output_trimmed(&output.stdout, "git config stdout").map_err(|message| {
                GitConfigGetError::InvalidOutput {
                    stream: "stdout",
                    message,
                }
            })?;
        return Ok(Some(stdout.to_string()));
    }
    let stderr =
        command_output_trimmed(&output.stderr, "git config stderr").map_err(|message| {
            GitConfigGetError::InvalidOutput {
                stream: "stderr",
                message,
            }
        })?;
    if output.status.code() == Some(1) && stderr.is_empty() {
        return Ok(None);
    }
    Err(GitConfigGetError::ReadFailed {
        status: exit_status_text(&output.status),
        stderr: stderr.to_string(),
    })
}

fn exit_status_text(status: &ExitStatus) -> String {
    status
        .code()
        .map(|code| format!("exit status {}", code))
        .unwrap_or_else(|| "terminated by signal".to_string())
}
