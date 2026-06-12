use std::io;
use std::path::Path;
use std::process::{Command, Output};

pub(crate) fn run_prompt_template_shell_command(root: &Path, command: &str) -> io::Result<Output> {
    Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(root)
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
