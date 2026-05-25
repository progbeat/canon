use crate::fs_util::ensure_dir_without_symlinks;
use crate::notes_header::validate_note_key;
use crate::output::{write_stderr_bytes, write_stdout_bytes};
use crate::project_types::Config;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::thread;

pub(crate) const INDEX_LOCK_STALE_AFTER_SECS: u64 = 600;

pub(crate) fn require_key(args: &[OsString], index: usize) -> Result<&str, String> {
    let key = args
        .get(index)
        .ok_or("missing key".to_string())
        .and_then(|arg| arg.to_str().ok_or("key must be valid UTF-8".to_string()))?;
    validate_note_key(key)?;
    Ok(key)
}

pub(crate) fn arg_to_string(arg: &OsString) -> Result<String, String> {
    arg.to_str()
        .map(|value| value.to_string())
        .ok_or("argument must be valid UTF-8".to_string())
}

pub(crate) fn collect_text(args: &[OsString], start: usize) -> Result<String, String> {
    let mut parts = Vec::new();
    let rest = args.get(start..).ok_or_else(|| {
        format!(
            "text start index {} exceeds argument count {}",
            start,
            args.len()
        )
    })?;
    for arg in rest {
        parts.push(arg.to_str().ok_or("text must be valid UTF-8".to_string())?);
    }
    Ok(parts.join(" "))
}

pub(crate) fn collect_text_or_stdin(args: &[OsString], start: usize) -> Result<String, String> {
    if args.len() > start {
        return collect_text(args, start);
    }
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|err| format!("failed to read stdin: {}", err))?;
    Ok(text)
}

pub(crate) fn run_rg(config: &Config, rg_args: &[OsString]) -> Result<(), String> {
    if rg_args.is_empty() {
        return Err("missing rg pattern".to_string());
    }
    ensure_dir_without_symlinks(&config.root)?;
    let mut command = Command::new("rg");
    command.args(rg_args);
    command.arg("--");
    command.arg(&config.root);
    let child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to run rg: {}", err))?;
    let mut child = ChildCleanup::new(child);
    let stdout = child
        .take_stdout()
        .ok_or_else(|| "failed to capture rg stdout".to_string())?;
    let stderr = child
        .take_stderr()
        .ok_or_else(|| "failed to capture rg stderr".to_string())?;
    let stdout_thread = thread::spawn(move || stream_rg_output(stdout, write_stdout_bytes));
    let stderr_thread = thread::spawn(move || stream_rg_output(stderr, write_stderr_bytes));
    let status = child
        .wait()
        .map_err(|err| format!("failed to wait for rg: {}", err))?;
    stdout_thread
        .join()
        .map_err(|_| "rg stdout streaming thread panicked".to_string())??;
    stderr_thread
        .join()
        .map_err(|_| "rg stderr streaming thread panicked".to_string())??;
    match status.code() {
        Some(0) | Some(1) => Ok(()),
        Some(code) => Err(format!("rg exited with status {}", code)),
        None => Err("rg terminated by signal".to_string()),
    }
}

struct ChildCleanup {
    child: Option<Child>,
}

impl ChildCleanup {
    fn new(child: Child) -> ChildCleanup {
        ChildCleanup { child: Some(child) }
    }

    fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.as_mut()?.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.as_mut()?.stderr.take()
    }

    fn wait(&mut self) -> Result<ExitStatus, std::io::Error> {
        let status = self
            .child
            .as_mut()
            .expect("rg child must exist before wait")
            .wait()?;
        self.child = None;
        Ok(status)
    }
}

impl Drop for ChildCleanup {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn stream_rg_output(
    reader: impl Read,
    mut write_chunk: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    // `rg` owns the search semantics, so canon treats each newline-delimited
    // byte line from the child pipe as the next known contiguous text segment.
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let bytes_read = reader
            .read_until(b'\n', &mut buffer)
            .map_err(|err| format!("failed to read rg output: {}", err))?;
        if bytes_read == 0 {
            return Ok(());
        }
        write_chunk(&buffer)?;
    }
}
