use crate::project::command_output_trimmed;
use std::io;
use std::path::Path;
use std::process::Command;

pub(crate) enum GitConfigGetError {
    Command(io::Error),
    InvalidOutput {
        stream: &'static str,
        message: String,
    },
    ReadFailed {
        key: String,
        stderr: String,
    },
}

pub(crate) fn git_config_get(root: &Path, key: &str) -> Result<Option<String>, GitConfigGetError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("config")
        .arg("--get")
        .arg(key)
        .output()
        .map_err(GitConfigGetError::Command)?;
    let stdout =
        command_output_trimmed(&output.stdout, "git config stdout").map_err(|message| {
            GitConfigGetError::InvalidOutput {
                stream: "stdout",
                message,
            }
        })?;
    let stderr =
        command_output_trimmed(&output.stderr, "git config stderr").map_err(|message| {
            GitConfigGetError::InvalidOutput {
                stream: "stderr",
                message,
            }
        })?;
    if output.status.success() {
        return Ok(Some(stdout.to_string()));
    }
    if stdout.is_empty() && stderr.is_empty() {
        return Ok(None);
    }
    Err(GitConfigGetError::ReadFailed {
        key: key.to_string(),
        stderr: stderr.to_string(),
    })
}

pub(crate) fn git_config_get_or_default<T, E>(
    root: &Path,
    key: &str,
    default: T,
    parse: impl FnOnce(&str) -> Result<T, E>,
    map_error: impl FnOnce(GitConfigGetError) -> E,
) -> Result<T, E> {
    match git_config_get(root, key).map_err(map_error)? {
        Some(value) => parse(&value),
        None => Ok(default),
    }
}
