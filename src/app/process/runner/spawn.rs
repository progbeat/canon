use super::child::{
    configure_app_server_environment, prepare_app_server_command, prepare_evaluator_codex_home,
    terminate_app_server_child,
};
use super::transport::{spawn_app_server_reader, spawn_app_server_stderr_reader};
use super::AppServerRunner;
use crate::config_types::AgentConfig;
use crate::evaluator::{
    app_server_args, EvaluatorError, EvaluatorProcessIsolation, EvaluatorProjectFilesystem,
    ReadOnlyProjectInspectionPlan,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

impl AppServerRunner {
    pub(super) fn new(
        load_plugins: bool,
        agent: &AgentConfig,
        inspection_plan: &ReadOnlyProjectInspectionPlan,
        project_filesystem: EvaluatorProjectFilesystem,
    ) -> Result<AppServerRunner, EvaluatorError> {
        let process_isolation = inspection_plan.process_isolation();
        let installed_codex_executable = resolve_codex_executable()?;
        // Plugin-enabled checked configs still run from Canon's isolated
        // Codex home; the checked tree must not select user-installed plugins
        // by making the app server inherit the caller's real home.
        let codex_home =
            prepare_evaluator_codex_home(process_isolation).map_err(EvaluatorError::message)?;
        let codex_executable = match process_isolation {
            EvaluatorProcessIsolation::CanonManaged => codex_home
                .materialize_runtime_executable(&installed_codex_executable)
                .map_err(EvaluatorError::message)?,
            EvaluatorProcessIsolation::ExternallyManaged => installed_codex_executable.clone(),
        };
        let mut command = Command::new(&codex_executable);
        let startup_args = app_server_args(
            &codex_executable,
            &installed_codex_executable,
            load_plugins,
            agent,
            inspection_plan,
        )
        .map_err(|err| EvaluatorError::message(err.to_string()))?;
        command.args(startup_args.args());
        let evaluator_runtime_root = codex_home.runtime_root().to_path_buf();
        configure_app_server_environment(&mut command, codex_home.path(), &evaluator_runtime_root)
            .map_err(EvaluatorError::message)?;
        prepare_app_server_command(&mut command);
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
            evaluator_runtime_inputs: Some(codex_home),
            child,
            stdin,
            messages,
            reader: Some(reader),
            stderr,
            stderr_reader: Some(stderr_reader),
            next_id: 1,
            token_usage_by_turn: BTreeMap::new(),
            latest_token_usage_by_thread: BTreeMap::new(),
            token_usage_updates_by_turn: BTreeMap::new(),
            context_compaction_events_by_turn: BTreeMap::new(),
            last_turn_usage: None,
            retired_threads: Default::default(),
            thread_runtime_inputs: BTreeMap::new(),
            progress: None,
            process_isolation,
            project_filesystem,
            startup_args: Some(startup_args),
            codex_executable,
        };
        runner.send_control_request(
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

fn resolve_codex_executable() -> Result<PathBuf, EvaluatorError> {
    let executable = which::which("codex").map_err(|err| {
        EvaluatorError::message(format!("failed to resolve codex executable: {}", err))
    })?;
    executable.canonicalize().map_err(|err| {
        EvaluatorError::message(format!(
            "failed to canonicalize codex executable {}: {}",
            executable.display(),
            err
        ))
    })
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
        self.startup_args.take();
        drop(self.evaluator_runtime_inputs.take());
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
