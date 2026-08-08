//! App-server request parameter serialization.

use crate::evaluator::{EphemeralEvaluatorThreadPermissionProfile, EvaluatorError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

const LOCAL_ENVIRONMENT_ID: &str = "local";
const SESSION_START_SOURCE: &str = "clear";

pub(in crate::app::process::runner) fn serialize_thread_start_params(
    context: &SerializedThreadStartParamsContext<'_>,
) -> Result<Value, EvaluatorError> {
    let cwd = path_to_json_string(context.cwd, "thread/start cwd")?;
    let params = ThreadStartParams {
        cwd: cwd.clone(),
        runtime_workspace_roots: vec![cwd.clone()],
        rendered_base_text: context.rendered_base_text,
        rendered_developer_text: context.rendered_developer_text,
        approval_policy: "never",
        permissions: context.permissions,
        sandbox: context.sandbox,
        environments: vec![local_environment_params(cwd)],
        config: context.config,
        ephemeral: true,
        dynamic_tools: context.dynamic_tools,
        session_start_source: SESSION_START_SOURCE,
    };
    serde_json::to_value(params).map_err(|err| {
        EvaluatorError::message(format!("failed to encode thread/start params: {err}"))
    })
}

pub(in crate::app::process::runner) struct SerializedThreadStartParamsContext<'a> {
    pub(in crate::app::process::runner) cwd: &'a Path,
    pub(in crate::app::process::runner) rendered_base_text: &'a str,
    pub(in crate::app::process::runner) rendered_developer_text: &'a str,
    pub(in crate::app::process::runner) permissions:
        Option<EphemeralEvaluatorThreadPermissionProfile>,
    pub(in crate::app::process::runner) sandbox: Option<&'a str>,
    pub(in crate::app::process::runner) config: &'a Value,
    pub(in crate::app::process::runner) dynamic_tools: &'a [Value],
}

pub(in crate::app::process::runner) fn thread_start_response_id(
    result: Value,
) -> Result<String, EvaluatorError> {
    let response: ThreadStartResponse = serde_json::from_value(result).map_err(|err| {
        EvaluatorError::message(format!("thread/start response missing thread.id: {err}"))
    })?;
    Ok(response.thread.id)
}

pub(in crate::app::process::runner) fn apply_local_turn_environment(
    request: &mut Value,
    cwd: &Path,
) -> Result<(), EvaluatorError> {
    let cwd = path_to_json_string(cwd, "turn/start cwd")?;
    request["cwd"] = Value::String(cwd.clone());
    request["runtimeWorkspaceRoots"] = json!([cwd.clone()]);
    request["environments"] = json!([local_environment_params(cwd)]);
    Ok(())
}

#[derive(Serialize)]
struct ThreadStartParams<'a> {
    cwd: String,
    #[serde(rename = "runtimeWorkspaceRoots")]
    runtime_workspace_roots: Vec<String>,
    #[serde(rename = "baseInstructions")]
    rendered_base_text: &'a str,
    #[serde(rename = "developerInstructions")]
    rendered_developer_text: &'a str,
    #[serde(rename = "approvalPolicy")]
    approval_policy: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    permissions: Option<EphemeralEvaluatorThreadPermissionProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sandbox: Option<&'a str>,
    environments: Vec<LocalEnvironmentParams>,
    config: &'a Value,
    ephemeral: bool,
    #[serde(rename = "dynamicTools", skip_serializing_if = "value_slice_is_empty")]
    dynamic_tools: &'a [Value],
    #[serde(rename = "sessionStartSource")]
    session_start_source: &'a str,
}

fn value_slice_is_empty(values: &[Value]) -> bool {
    values.is_empty()
}

#[derive(Serialize)]
struct LocalEnvironmentParams {
    #[serde(rename = "environmentId")]
    environment_id: &'static str,
    cwd: String,
    #[serde(rename = "runtimeWorkspaceRoots")]
    runtime_workspace_roots: Vec<String>,
}

fn local_environment_params(cwd: String) -> LocalEnvironmentParams {
    LocalEnvironmentParams {
        environment_id: LOCAL_ENVIRONMENT_ID,
        cwd: cwd.clone(),
        runtime_workspace_roots: vec![cwd],
    }
}

fn path_to_json_string(path: &Path, context: &'static str) -> Result<String, EvaluatorError> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| EvaluatorError::message(format!("{context} must be valid UTF-8")))
}

#[derive(Deserialize)]
struct ThreadStartResponse {
    thread: ThreadStartThread,
}

#[derive(Deserialize)]
struct ThreadStartThread {
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: hQ,KD
    fn local_runtime_workspace_matches_the_app_server_request_contract() {
        let config = json!({});
        let thread_request = serialize_thread_start_params(&SerializedThreadStartParamsContext {
            cwd: Path::new("/cwd"),
            rendered_base_text: "base",
            rendered_developer_text: "developer",
            permissions: None,
            sandbox: None,
            config: &config,
            dynamic_tools: &[],
        })
        .unwrap();
        let mut turn_request = json!({});
        apply_local_turn_environment(&mut turn_request, Path::new("/cwd")).unwrap();

        // The names and nesting below are Codex's public app-server wire
        // contract. Both request kinds must identify the same local runtime
        // root or command execution can select a different working directory.
        for request in [&thread_request, &turn_request] {
            assert_eq!(request["cwd"], json!("/cwd"));
            assert_eq!(request["runtimeWorkspaceRoots"], json!(["/cwd"]));
            assert_eq!(
                request["environments"],
                json!([{
                    "environmentId": "local",
                    "cwd": "/cwd",
                    "runtimeWorkspaceRoots": ["/cwd"],
                }])
            );
        }
    }
}
