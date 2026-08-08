use crate::output::command_output_trimmed;
use std::fmt;
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

impl fmt::Display for GitConfigGetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitConfigGetError::Command(err) => write!(formatter, "failed to run git config: {err}"),
            GitConfigGetError::InvalidOutput { stream, message } => {
                write!(formatter, "invalid git config {stream}: {message}")
            }
            GitConfigGetError::ReadFailed { status, stderr, .. } => {
                write!(formatter, "git config failed with {status}: {stderr}")
            }
        }
    }
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
        .args(["--null", "--get"])
        .arg(key)
        .output()
        .map_err(GitConfigGetError::Command)?;
    if output.status.success() {
        let stdout = nul_terminated_config_value(&output.stdout).map_err(|message| {
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

fn nul_terminated_config_value(bytes: &[u8]) -> Result<&str, String> {
    let value = bytes
        .strip_suffix(b"\0")
        .ok_or_else(|| "git config value must be NUL-terminated".to_string())?;
    if value.contains(&0) {
        return Err("git config returned more than one value".to_string());
    }
    std::str::from_utf8(value).map_err(|err| format!("git config value must be valid UTF-8: {err}"))
}

fn exit_status_text(status: &ExitStatus) -> String {
    status
        .code()
        .map(|code| format!("exit status {}", code))
        .unwrap_or_else(|| "terminated by signal".to_string())
}

#[cfg(test)]
mod tests {
    use super::nul_terminated_config_value;

    #[test] // xpec: gO
    fn nul_terminated_git_config_value_preserves_all_content() {
        assert_eq!(
            nul_terminated_config_value(b" value \n\0").unwrap(),
            " value \n"
        );
    }

    #[test] // xpec: gO
    fn nul_terminated_git_config_value_requires_exactly_one_record() {
        assert!(nul_terminated_config_value(b"value").is_err());
        assert!(nul_terminated_config_value(b"one\0two\0").is_err());
    }
}
