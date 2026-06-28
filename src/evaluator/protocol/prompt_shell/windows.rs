pub(crate) use super::posix::quote_prompt_template_shell_arg;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub(crate) fn run_prompt_template_shell_command(
    root: &Path,
    shell_command: &str,
    env: &[(&str, &str)],
) -> io::Result<Output> {
    match super::posix::run_prompt_template_shell_command(root, shell_command, env) {
        Ok(output) => Ok(output),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let Some(shell) = git_for_windows_sh() else {
                return Err(err);
            };
            run_prompt_template_shell_command_with(shell, root, shell_command, env)
        }
        Err(err) => Err(err),
    }
}

fn run_prompt_template_shell_command_with(
    shell: impl AsRef<std::ffi::OsStr>,
    root: &Path,
    shell_command: &str,
    env: &[(&str, &str)],
) -> io::Result<Output> {
    let mut command = Command::new(shell);
    command
        .arg("-c")
        .arg(shell_command)
        .current_dir(root)
        .envs(env.iter().copied())
        .output()
}

fn git_for_windows_sh() -> Option<PathBuf> {
    let output = Command::new("git").arg("--exec-path").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let exec_path = String::from_utf8(output.stdout).ok()?;
    git_for_windows_sh_from_exec_path(Path::new(exec_path.trim()))
}

fn git_for_windows_sh_from_exec_path(exec_path: &Path) -> Option<PathBuf> {
    for ancestor in exec_path.ancestors() {
        let candidate = ancestor.join("usr").join("bin").join("sh.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
