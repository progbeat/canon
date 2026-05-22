use crate::fs_util::ensure_dir_without_symlinks;
use crate::git::resolve_git_path;
use crate::output::write_stdout_line;
use crate::project_types::Config;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn print_root(config: &Config) -> Result<(), String> {
    ensure_dir_without_symlinks(&config.root)?;
    write_stdout_line(&config.root.display().to_string())
}

impl Config {
    pub(crate) fn from_env() -> Result<Config, String> {
        let thread_id = env::var("CODEX_THREAD_ID")
            .map_err(|_| "CODEX_THREAD_ID is required in v1".to_string())?;
        if thread_id.trim().is_empty() {
            return Err("CODEX_THREAD_ID is empty".to_string());
        }
        if thread_id == "."
            || thread_id == ".."
            || thread_id.contains('/')
            || thread_id.contains('\\')
        {
            return Err("CODEX_THREAD_ID must be a single path segment".to_string());
        }

        #[cfg(test)]
        if let Some(value) = env::var_os("CANON_HOME") {
            if !value.is_empty() {
                return Ok(Config {
                    root: PathBuf::from(value)
                        .join(".git")
                        .join("canon")
                        .join("codex")
                        .join(thread_id),
                });
            }
        }

        let current_dir =
            env::current_dir().map_err(|err| format!("failed to read current dir: {}", err))?;
        let project_root = git_project_root(&current_dir)?;
        Config::for_project_thread(&project_root, &thread_id)
    }

    pub(crate) fn for_project_thread(root: &Path, thread_id: &str) -> Result<Config, String> {
        let state_root = resolve_git_path(root, "canon")?;
        // Notes are intentionally thread-scoped retained data. Appends under a
        // thread root are small note/index log records, and those logs are
        // threshold-compacted after enough appended bytes accumulate to pay for
        // the rewrite amortized. The practical state bound is therefore
        // conditional on a bounded retained set of thread roots and note keys;
        // automatic cleanup must not delete those user-retained notes.
        Ok(Config {
            root: state_root.join("codex").join(thread_id),
        })
    }
}

pub(crate) fn project_root_or_current(start: &Path) -> Result<PathBuf, String> {
    match git_project_root(start) {
        Ok(root) => Ok(root),
        Err(_) => env::current_dir().map_err(|err| format!("failed to read current dir: {}", err)),
    }
}

pub(crate) fn command_output_trimmed<'a>(
    bytes: &'a [u8],
    description: &str,
) -> Result<&'a str, String> {
    Ok(std::str::from_utf8(bytes)
        .map_err(|err| format!("{} must be valid UTF-8: {}", description, err))?
        .trim())
}

pub(crate) fn git_project_root(start: &Path) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(start)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .map_err(|err| format!("failed to run git rev-parse: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to find git project root: {}",
            command_output_trimmed(&output.stderr, "git rev-parse stderr")?
        ));
    }
    Ok(path_from_git_stdout(output.stdout))
}

pub(crate) fn path_from_git_stdout(mut bytes: Vec<u8>) -> PathBuf {
    while matches!(bytes.last(), Some(b'\n' | b'\r')) {
        bytes.pop();
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(bytes))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(&bytes).to_string())
    }
}
