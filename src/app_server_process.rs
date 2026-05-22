use crate::app_server::AppServerRunner;
use crate::config_types::AgentConfig;
use crate::evaluator_config::app_server_args;
use crate::evaluator_types::EvaluatorError;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};

#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::os::unix::fs::symlink;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::process::ExitStatus;
#[cfg(unix)]
use std::time::{Duration, Instant};

const EVALUATOR_CODEX_HOME_AUTH_FILES: &[&str] = &["auth.json", "installation_id", "version.json"];
const SYSTEM_SKILLS_MARKER: &str = ".codex-system-skills.marker";
const EVALUATOR_CODEX_HOME_RESET_DIRS: &[&str] =
    &["mcp", "memories", "plugins", "sessions", "skills"];
const EVALUATOR_CODEX_HOME_RESET_FILES: &[&str] = &[
    "AGENTS.md",
    "config.json",
    "config.toml",
    "instructions.md",
    "preferences.json",
];

pub(crate) fn spawn_app_server_reader(
    stdout: std::process::ChildStdout,
) -> (Receiver<Result<Value, String>>, JoinHandle<()>) {
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match stdout.read_line(&mut line) {
                Ok(0) => return,
                Ok(_) => {
                    let parsed = serde_json::from_str(line.trim_end())
                        .map_err(|err| format!("failed to parse app-server JSON: {}", err));
                    if sender.send(parsed).is_err() {
                        return;
                    }
                }
                Err(err) => {
                    let _ =
                        sender.send(Err(format!("failed to read app-server response: {}", err)));
                    return;
                }
            }
        }
    });
    (receiver, reader)
}

pub(crate) fn spawn_app_server_stderr_reader(
    mut stderr: std::process::ChildStderr,
) -> (Receiver<String>, JoinHandle<()>) {
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut text = String::new();
        if stderr.read_to_string(&mut text).is_ok() && !text.trim().is_empty() {
            let _ = sender.send(text);
        }
    });
    (receiver, reader)
}

impl AppServerRunner {
    pub(crate) fn new(
        root: &Path,
        load_plugins: bool,
        agent: &AgentConfig,
    ) -> Result<AppServerRunner, EvaluatorError> {
        let mut command = Command::new("codex");
        command.args(app_server_args(root, load_plugins, agent)?);
        if !load_plugins {
            let codex_home = prepare_evaluator_codex_home(root).map_err(EvaluatorError::message)?;
            command.env("CODEX_HOME", &codex_home);
        }
        // `canon check` can itself run inside a Codex thread. The evaluator
        // app-server must create independent invocation-local threads, not
        // attach to the parent conversation through inherited thread identity.
        command.env_remove("CODEX_THREAD_ID");
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("failed to start codex app-server: {}", err))?;
        let stdin = take_child_pipe(
            &mut child,
            |child| child.stdin.take(),
            "failed to open app-server stdin",
        )?;
        let stdout = take_child_pipe(
            &mut child,
            |child| child.stdout.take(),
            "failed to open app-server stdout",
        )?;
        let stderr = take_child_pipe(
            &mut child,
            |child| child.stderr.take(),
            "failed to open app-server stderr",
        )?;
        let (messages, reader) = spawn_app_server_reader(stdout);
        let (stderr, stderr_reader) = spawn_app_server_stderr_reader(stderr);
        let mut runner = AppServerRunner {
            child,
            stdin,
            messages,
            reader: Some(reader),
            stderr,
            stderr_reader: Some(stderr_reader),
            next_id: 1,
            token_usage_by_turn: BTreeMap::new(),
            token_usage_updates_by_turn: BTreeMap::new(),
            context_compaction_events_by_turn: BTreeMap::new(),
            last_turn_usage: None,
            retired_sessions: Default::default(),
        };
        runner.send_request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "canon",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "experimentalApi": true
                }
            }),
        )?;
        Ok(runner)
    }
}

pub(crate) fn prepare_evaluator_codex_home(_root: &Path) -> Result<PathBuf, String> {
    let codex_home = evaluator_codex_home_path()?;
    ensure_evaluator_codex_home_dir(&codex_home)?;
    for file in EVALUATOR_CODEX_HOME_RESET_FILES {
        remove_existing_codex_home_entry(&codex_home.join(file))?;
    }
    for dir in EVALUATOR_CODEX_HOME_RESET_DIRS {
        remove_existing_codex_home_entry(&codex_home.join(dir))?;
    }
    for dir in [
        ".tmp", "cache", "log", "mcp", "memories", "plugins", "sessions", "skills",
    ] {
        ensure_evaluator_codex_home_dir(&codex_home.join(dir))?;
    }
    let source_home = source_codex_home();
    write_empty_system_skills_marker(source_home.as_deref(), &codex_home)?;
    if let Some(source_home) = source_home {
        if source_home != codex_home {
            for file_name in EVALUATOR_CODEX_HOME_AUTH_FILES {
                mirror_codex_home_file(&source_home, &codex_home, file_name)?;
            }
        }
    }
    Ok(codex_home)
}

fn evaluator_codex_home_path() -> Result<PathBuf, String> {
    let temp_root = env::temp_dir()
        .canonicalize()
        .map_err(|err| format!("failed to canonicalize temp dir: {}", err))?;
    Ok(temp_root.join("canon").join(".codex"))
}

fn ensure_evaluator_codex_home_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|err| format!("failed to create {}: {}", path.display(), err))
}

fn write_empty_system_skills_marker(
    source_home: Option<&Path>,
    target_home: &Path,
) -> Result<(), String> {
    let system_dir = target_home.join("skills").join(".system");
    ensure_evaluator_codex_home_dir(&system_dir)?;
    let target = system_dir.join(SYSTEM_SKILLS_MARKER);
    if let Some(source) = source_home.map(|source_home| {
        source_home
            .join("skills")
            .join(".system")
            .join(SYSTEM_SKILLS_MARKER)
    }) {
        if source.is_file() {
            fs::copy(&source, &target).map_err(|err| {
                format!(
                    "failed to copy evaluator system skills marker {} from {}: {}",
                    target.display(),
                    source.display(),
                    err
                )
            })?;
            return Ok(());
        }
    }
    fs::write(&target, b"canon-empty-system-skills\n")
        .map_err(|err| format!("failed to write {}: {}", target.display(), err))
}

fn source_codex_home() -> Option<PathBuf> {
    env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
}

fn mirror_codex_home_file(
    source_home: &Path,
    target_home: &Path,
    file_name: &str,
) -> Result<(), String> {
    let source = source_home.join(file_name);
    if !source.is_file() {
        return Ok(());
    }
    let target = target_home.join(file_name);
    remove_existing_codex_home_entry(&target)?;
    #[cfg(unix)]
    {
        symlink(&source, &target).map_err(|err| {
            format!(
                "failed to symlink evaluator CODEX_HOME file {} to {}: {}",
                target.display(),
                source.display(),
                err
            )
        })?;
    }
    #[cfg(not(unix))]
    {
        fs::copy(&source, &target).map_err(|err| {
            format!(
                "failed to copy evaluator CODEX_HOME file {} from {}: {}",
                target.display(),
                source.display(),
                err
            )
        })?;
    }
    Ok(())
}

fn remove_existing_codex_home_entry(path: &Path) -> Result<(), String> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|err| format!("failed to replace {}: {}", path.display(), err))
}

impl Drop for AppServerRunner {
    fn drop(&mut self) {
        let _ = terminate_app_server_child(&mut self.child);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

fn cleanup_error_after_missing_pipe(child: &mut Child, message: &str) -> EvaluatorError {
    match terminate_app_server_child(child) {
        Ok(()) => EvaluatorError::message(message),
        Err(err) => EvaluatorError::message(format!("{}; cleanup failed: {}", message, err)),
    }
}

fn take_child_pipe<T>(
    child: &mut Child,
    take: impl FnOnce(&mut Child) -> Option<T>,
    message: &str,
) -> Result<T, EvaluatorError> {
    take(child).ok_or_else(|| cleanup_error_after_missing_pipe(child, message))
}

#[cfg(unix)]
pub(crate) fn terminate_app_server_child(child: &mut Child) -> Result<(), String> {
    if poll_app_server_child(child)?.is_some() {
        return Ok(());
    }
    let process_group = child.id() as i32;
    let mut errors = Vec::new();
    signal_process_group_or_kill_child(child, process_group, 15, &mut errors);
    if wait_for_child_exit(child, Duration::from_secs(2))? {
        return finish_app_server_cleanup(errors);
    }
    signal_process_group_or_kill_child(child, process_group, 9, &mut errors);
    wait_for_app_server_child(child)?;
    finish_app_server_cleanup(errors)
}

#[cfg(unix)]
pub(crate) fn signal_process_group(process_group: i32, signal_number: i32) -> Result<(), String> {
    // SAFETY: POSIX `kill` uses a negative pid to address a process group.
    // The caller passes the child pid as the process-group id after spawning
    // the app-server child in its own group; the return value is checked.
    let result = unsafe { crate::kill(-process_group, signal_number) };
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "failed to send signal {} to app-server process group {}: {}",
            signal_number,
            process_group,
            io::Error::last_os_error()
        ))
    }
}

#[cfg(unix)]
fn signal_process_group_or_kill_child(
    child: &mut Child,
    process_group: i32,
    signal_number: i32,
    errors: &mut Vec<String>,
) {
    if let Err(err) = signal_process_group(process_group, signal_number) {
        errors.push(err);
        if let Err(err) = child.kill() {
            errors.push(format!("failed to kill app-server child: {}", err));
        }
    }
}

#[cfg(unix)]
pub(crate) fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> Result<bool, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if poll_app_server_child(child)?.is_some() {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn poll_app_server_child(child: &mut Child) -> Result<Option<ExitStatus>, String> {
    child
        .try_wait()
        .map_err(|err| format!("failed to poll app-server child: {}", err))
}

#[cfg(unix)]
fn finish_app_server_cleanup(errors: Vec<String>) -> Result<(), String> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(not(unix))]
pub(crate) fn terminate_app_server_child(child: &mut Child) -> Result<(), String> {
    child
        .kill()
        .map_err(|err| format!("failed to kill app-server child: {}", err))?;
    wait_for_app_server_child(child)?;
    Ok(())
}

fn wait_for_app_server_child(child: &mut Child) -> Result<(), String> {
    child
        .wait()
        .map(|_| ())
        .map_err(|err| format!("failed to wait for app-server child: {}", err))
}
