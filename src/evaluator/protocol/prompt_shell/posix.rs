use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{Command, Output};

pub(crate) fn run_prompt_template_shell_command(
    root: &Path,
    shell_command: &str,
    env: &[(OsString, OsString)],
    args: &[String],
) -> io::Result<Output> {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(shell_command)
        .arg("canon-prompt-template")
        .args(args)
        .current_dir(root)
        .envs(env.iter().map(|(key, value)| (key, value)))
        .output()
}

pub(crate) fn quote_prompt_template_shell_arg(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Ok("''".to_string());
    }
    let mut quoted = String::new();
    quoted.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    Ok(quoted)
}
