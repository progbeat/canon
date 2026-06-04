use crate::app::server::AppServerRunner;
use crate::config_types::AgentConfig;
use crate::evaluator::{app_server_args_with_no_sandbox, EvaluatorError};
use crate::fs_util::ensure_dir_without_symlinks;
use crate::git::resolve_git_path;
use crate::platform;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};

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
    stderr: std::process::ChildStderr,
) -> (Receiver<String>, JoinHandle<()>) {
    spawn_app_server_stderr_reader_with_forwarder(stderr, |_| Ok(()))
}

pub(crate) fn spawn_app_server_stderr_reader_with_forwarder<R, F>(
    mut stderr: R,
    forward: F,
) -> (Receiver<String>, JoinHandle<()>)
where
    R: Read + Send + 'static,
    F: Fn(&[u8]) -> Result<(), String> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match stderr.read(&mut buffer) {
                Ok(0) => return,
                Ok(n) => {
                    let bytes = &buffer[..n];
                    let _ = forward(bytes);
                    let _ = sender.send(String::from_utf8_lossy(bytes).into_owned());
                }
                Err(_) => return,
            }
        }
    });
    (receiver, reader)
}

impl AppServerRunner {
    pub(crate) fn new(
        root: &Path,
        load_plugins: bool,
        agent: &AgentConfig,
        no_sandbox: bool,
    ) -> Result<AppServerRunner, EvaluatorError> {
        let mut command = Command::new("codex");
        command.args(app_server_args_with_no_sandbox(
            root,
            load_plugins,
            agent,
            no_sandbox,
        )?);
        // Plugin-enabled checked configs still run from Canon's isolated
        // Codex home; the checked tree must not select user-installed plugins
        // by making the app server inherit the caller's real home.
        let codex_home = prepare_evaluator_codex_home(root).map_err(EvaluatorError::message)?;
        configure_app_server_environment(&mut command, &codex_home)
            .map_err(EvaluatorError::message)?;
        platform::prepare_app_server_command(&mut command);
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
            app_server_root: root.to_path_buf(),
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
            session_cwds: BTreeMap::new(),
            no_sandbox,
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

pub(crate) fn configure_app_server_environment(
    command: &mut Command,
    isolated_codex_home: &Path,
) -> Result<(), String> {
    let path = env::var_os("PATH");
    let temp_root = evaluator_temp_root()?;
    command.env_clear();
    if let Some(path) = path {
        command.env("PATH", path);
    }
    command.env("CODEX_HOME", isolated_codex_home);
    if let Some(home) = isolated_codex_home.parent() {
        command.env("HOME", home);
    }
    for key in ["TMPDIR", "TEMP", "TMP"] {
        command.env(key, &temp_root);
    }
    Ok(())
}

pub(crate) fn prepare_evaluator_codex_home(root: &Path) -> Result<PathBuf, String> {
    let codex_home = evaluator_codex_home_path(root)?;
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
        if !same_existing_path(&source_home, &codex_home) {
            for file_name in EVALUATOR_CODEX_HOME_AUTH_FILES {
                mirror_codex_home_file(&source_home, &codex_home, file_name)?;
            }
        }
    }
    Ok(codex_home)
}

fn evaluator_codex_home_path(root: &Path) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|err| format!("failed to canonicalize evaluator root: {}", err))?;
    resolve_git_path(&root, "canon/evaluator-codex-home/.codex")
}

fn evaluator_temp_root() -> Result<PathBuf, String> {
    env::temp_dir()
        .canonicalize()
        .map_err(|err| format!("failed to canonicalize temp dir: {}", err))
}

fn ensure_evaluator_codex_home_dir(path: &Path) -> Result<(), String> {
    ensure_dir_without_symlinks(path)
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
    if same_existing_path(&source, &target) {
        return Ok(());
    }
    remove_existing_codex_home_entry(&target)?;
    platform::mirror_evaluator_codex_home_file(&source, &target)
}

fn same_existing_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
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
        let _ = platform::terminate_app_server_child(&mut self.child);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

fn cleanup_error_after_missing_pipe(child: &mut Child, message: &str) -> EvaluatorError {
    match platform::terminate_app_server_child(child) {
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    #[test]
    fn stderr_reader_forwards_before_eof() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let (forwarded_tx, forwarded_rx) = mpsc::channel();
        let (captured_rx, handle) =
            spawn_app_server_stderr_reader_with_forwarder(reader, move |bytes| {
                forwarded_tx
                    .send(bytes.to_vec())
                    .map_err(|err| err.to_string())
            });

        writer.write_all(b"early stderr\n").unwrap();

        assert_eq!(
            forwarded_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            b"early stderr\n"
        );
        assert_eq!(
            captured_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "early stderr\n"
        );

        drop(writer);
        handle.join().unwrap();
    }

    #[test]
    fn app_server_environment_uses_isolated_codex_home() {
        let mut command = Command::new("codex");
        let codex_home = Path::new("/tmp/canon-evaluator-home/.codex");

        configure_app_server_environment(&mut command, codex_home).unwrap();

        assert_eq!(
            command_env_value(&command, "CODEX_HOME"),
            Some(codex_home.as_os_str().to_os_string())
        );
        assert_eq!(
            command_env_value(&command, "HOME"),
            Some(codex_home.parent().unwrap().as_os_str().to_os_string())
        );
    }

    fn command_env_value(command: &Command, key: &str) -> Option<OsString> {
        command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new(key))
            .and_then(|(_, value)| value.map(OsStr::to_os_string))
    }
}
