use std::io;
use std::path::Path;
use std::process::{Command, Output};

pub(crate) fn run_prompt_template_shell_command(root: &Path, command: &str) -> io::Result<Output> {
    Command::new("cmd")
        .arg("/D")
        .arg("/V:OFF")
        .arg("/C")
        .arg(command)
        .current_dir(root)
        .output()
}

pub(crate) fn quote_prompt_template_shell_arg(value: &str) -> Result<String, String> {
    reject_unquotable_cmd_arg(value)?;
    if value.is_empty() {
        return Ok("\"\"".to_string());
    }
    let mut quoted = String::new();
    quoted.push('"');
    for ch in value.chars() {
        match ch {
            '^' => quoted.push_str("^^"),
            '%' => quoted.push_str("^%"),
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    Ok(quoted)
}

fn reject_unquotable_cmd_arg(value: &str) -> Result<(), String> {
    if value.contains('\0') {
        return Err("shell argument cannot contain NUL bytes".to_string());
    }
    if value.contains('\r') || value.contains('\n') {
        return Err("shell argument cannot contain newlines on Windows".to_string());
    }
    if value.contains('"') {
        return Err("shell argument cannot contain double quotes on Windows".to_string());
    }
    Ok(())
}
