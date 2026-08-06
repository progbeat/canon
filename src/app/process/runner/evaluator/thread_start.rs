//! Evaluator thread-start preparation and memoization for one runner invocation.

use super::super::transport::{
    serialize_thread_start_params, thread_start_response_id, SerializedThreadStartParamsContext,
};
use super::super::AppServerRunner;
use super::process_isolation::thread_permission_selection;
use crate::config_types::AgentConfig;
use crate::evaluator::{
    EvaluatorError, EvaluatorHostIsolation, EvaluatorProcessIsolation,
    EvaluatorRuntimeConfigContext, EvaluatorRuntimeConfigSnapshot,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

#[derive(Default)]
pub(in crate::app::process::runner) struct InvocationThreadStartMemo {
    runtime_configs: HashMap<EvaluatorRuntimeConfigSnapshot, Result<Value, String>>,
    serialized_params: HashMap<AppServerThreadStartKey, Result<Value, String>>,
}

pub(in crate::app::process::runner) struct AppServerThreadStartContext<'a> {
    pub(in crate::app::process::runner) cwd: &'a Path,
    pub(in crate::app::process::runner) template_artifact_directory: &'a Path,
    pub(in crate::app::process::runner) host_isolation: &'a EvaluatorHostIsolation,
    pub(in crate::app::process::runner) rendered_base_text: &'a str,
    pub(in crate::app::process::runner) rendered_developer_text: &'a str,
    pub(in crate::app::process::runner) agent: &'a AgentConfig,
    pub(in crate::app::process::runner) model: Option<&'a str>,
    pub(in crate::app::process::runner) thinking: &'a str,
    pub(in crate::app::process::runner) app_server_state_root: Option<&'a Path>,
    pub(in crate::app::process::runner) process_isolation: EvaluatorProcessIsolation,
    pub(in crate::app::process::runner) dynamic_tools: &'a [Value],
    pub(in crate::app::process::runner) codex_executable: &'a Path,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct AppServerThreadStartKey {
    rendered_base_text: String,
    rendered_developer_text: String,
    // Runtime configuration identity is owned and evolved by the evaluator.
    runtime_config_snapshot: EvaluatorRuntimeConfigSnapshot,
    dynamic_tools: Vec<Value>,
}

impl InvocationThreadStartMemo {
    pub(in crate::app::process::runner) fn resolve(
        &mut self,
        context: AppServerThreadStartContext<'_>,
    ) -> Result<Value, EvaluatorError> {
        let key = AppServerThreadStartKey::capture(context);
        if let Some(params) = self.serialized_params.get(&key) {
            return params.clone().map_err(EvaluatorError::message);
        }
        let config = if let Some(config) = self.runtime_configs.get(&key.runtime_config_snapshot) {
            config.clone()
        } else {
            let config = key
                .runtime_config_snapshot
                .to_json_value()
                .map_err(|error| error.to_string());
            self.runtime_configs
                .insert(key.runtime_config_snapshot.clone(), config.clone());
            config
        };
        let params =
            config.and_then(|config| key.serialize(&config).map_err(|error| error.to_string()));
        self.serialized_params.insert(key, params.clone());
        params.map_err(EvaluatorError::message)
    }
}

impl AppServerThreadStartKey {
    fn capture(context: AppServerThreadStartContext<'_>) -> AppServerThreadStartKey {
        let runtime_config_snapshot =
            EvaluatorRuntimeConfigSnapshot::capture(EvaluatorRuntimeConfigContext {
                agent: context.agent,
                model: context.model,
                thinking: context.thinking,
                app_server_state_root: context.app_server_state_root,
                session_root: context.cwd,
                template_artifact_directory: context.template_artifact_directory,
                host_isolation: context.host_isolation,
                process_isolation: context.process_isolation,
                codex_executable: context.codex_executable,
            });
        AppServerThreadStartKey {
            rendered_base_text: context.rendered_base_text.to_string(),
            rendered_developer_text: context.rendered_developer_text.to_string(),
            runtime_config_snapshot,
            dynamic_tools: context.dynamic_tools.to_vec(),
        }
    }

    fn serialize(&self, config: &Value) -> Result<Value, EvaluatorError> {
        let permission_selection =
            thread_permission_selection(self.runtime_config_snapshot.process_isolation());
        serialize_thread_start_params(&SerializedThreadStartParamsContext {
            cwd: self.runtime_config_snapshot.session_root(),
            rendered_base_text: &self.rendered_base_text,
            rendered_developer_text: &self.rendered_developer_text,
            permissions: permission_selection.permissions,
            sandbox: permission_selection.sandbox,
            config,
            dynamic_tools: &self.dynamic_tools,
        })
    }
}

impl AppServerRunner {
    pub(in crate::app::process::runner) fn start_thread(
        &mut self,
        thread_cwd: &Path,
        template_artifact_directory: &Path,
        project_tools_advertised: bool,
        params: Value,
    ) -> Result<String, EvaluatorError> {
        // thread/start creates the evaluator agent, and `thread_cwd` is that
        // agent's working directory. In Git-backed mode it is the materialized
        // checked tree; in in-place mode it is the checked directory itself.
        // The already-running app-server transport process has its own inert
        // temporary cwd, configured independently in `child/environment.rs`.
        let result = self.send_control_request("thread/start", params)?;
        let thread_id = thread_start_response_id(result)?;
        self.remember_thread_runtime_inputs(
            thread_id.clone(),
            thread_cwd.to_path_buf(),
            template_artifact_directory.to_path_buf(),
            project_tools_advertised,
        );
        Ok(thread_id)
    }
}

#[cfg(test)]
mod tests;
