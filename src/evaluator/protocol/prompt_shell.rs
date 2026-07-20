use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::Output;

mod posix;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

pub(crate) fn run_prompt_template_shell_command(
    root: &Path,
    command: &str,
    env: &[(OsString, OsString)],
    args: &[String],
) -> io::Result<Output> {
    imp::run_prompt_template_shell_command(root, command, env, args)
}

pub(crate) fn quote_prompt_template_shell_arg(value: &str) -> Result<String, String> {
    imp::quote_prompt_template_shell_arg(value)
}
