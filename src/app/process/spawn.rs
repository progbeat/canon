use super::{
    configure_app_server_environment, prepare_app_server_command, prepare_evaluator_codex_home,
    spawn_app_server_reader, spawn_app_server_stderr_reader, terminate_app_server_child,
    AppServerRunner,
};
use crate::config_types::AgentConfig;
use crate::evaluator::{app_server_args_with_no_sandbox, EvaluatorError};
use serde_json::json;
use std::collections::BTreeMap;
use std::env;
use std::path::Path;
use std::process::{Child, Command, Stdio};

impl AppServerRunner {
    pub(crate) fn new(
        root: &Path,
        load_plugins: bool,
        agent: &AgentConfig,
        no_sandbox: bool,
    ) -> Result<AppServerRunner, EvaluatorError> {
        let mut command = Command::new("codex");
        let app_server_args =
            app_server_args_with_no_sandbox(root, load_plugins, agent, no_sandbox)
                .map_err(|err| EvaluatorError::message(err.to_string()))?;
        command.args(&app_server_args.args);
        let mut model_catalog_file = app_server_args.model_catalog_file;
        // Plugin-enabled checked configs still run from Canon's isolated
        // Codex home; the checked tree must not select user-installed plugins
        // by making the app server inherit the caller's real home.
        let codex_home = prepare_evaluator_codex_home(root).map_err(EvaluatorError::message)?;
        configure_app_server_environment(&mut command, &codex_home)
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
            progress: None,
            no_sandbox,
            startup_model_catalog_file: model_catalog_file.take(),
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

impl Drop for AppServerRunner {
    fn drop(&mut self) {
        let _ = terminate_app_server_child(&mut self.child);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
        self.startup_model_catalog_file.take();
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
