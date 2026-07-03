use super::{AppServerRunner, AppServerTurnRequest};
use crate::check::codex_reasoning_effort;
use crate::config_types::AgentConfig;
use crate::evaluator::{
    evaluator_thread_config_with_no_sandbox, EvaluatorDynamicToolHandler, EvaluatorError,
    EvaluatorProgress, EvaluatorRunner,
};
use crate::token_usage_types::EvaluatorTurnUsage;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const EVALUATOR_SESSION_START_SOURCE: &str = "clear";
const LOCAL_ENVIRONMENT_ID: &str = "local";

impl EvaluatorRunner for AppServerRunner {
    fn start_session(
        &mut self,
        session_cwd: &Path,
        template_artifact_paths: &[PathBuf],
        base_instructions: &str,
        developer_instructions: &str,
        agent: &AgentConfig,
        model: Option<&str>,
        thinking: &str,
        scope: &[String],
        dynamic_tools: &[Value],
    ) -> Result<String, EvaluatorError> {
        let session_cwd_json = path_to_json_string(session_cwd, "thread/start cwd")?;
        // `session_cwd` is the staged Git snapshot root supplied by
        // `check_interrogation`; it is distinct from `LazyAppServerRunner`'s
        // app-server startup root, which is the real project root used for
        // Canon runtime state and app-server configuration.
        let params = ThreadStartParams {
            cwd: session_cwd_json,
            base_instructions,
            developer_instructions,
            approval_policy: "never",
            sandbox: Some(thread_start_sandbox_mode(self.no_sandbox())),
            environments: vec![local_environment_params(session_cwd)?],
            config: evaluator_thread_config_with_no_sandbox(
                agent,
                scope,
                model,
                thinking,
                self.app_server_state_root(),
                session_cwd,
                template_artifact_paths,
                self.no_sandbox(),
            )
            .map_err(|err| EvaluatorError::message(err.to_string()))?,
            // Evaluator threads are invocation-local and ephemeral. Canon still
            // reuses live thread IDs by scope within this `canon check`, but
            // oversized carryover is handled by retiring the local session ID
            // so the next same-scope interrogation starts a fresh thread.
            ephemeral: true,
            dynamic_tools,
            // Evaluator threads must not inherit the parent Codex conversation:
            // canon questions about "your dev instructions" refer only to the
            // rendered evaluator developerInstructions parameter below.
            session_start_source: EVALUATOR_SESSION_START_SOURCE,
        };
        let params = serde_json::to_value(params)
            .map_err(|err| format!("failed to encode thread/start params: {}", err))?;
        let result = self.send_control_request("thread/start", params)?;
        let response: ThreadStartResponse = serde_json::from_value(result)
            .map_err(|err| format!("thread/start response missing thread.id: {}", err))?;
        self.remember_session_cwd(response.thread.id.clone(), session_cwd.to_path_buf());
        Ok(response.thread.id)
    }

    fn ask(
        &mut self,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
        thinking: &str,
        output_schema: &Value,
        dynamic_tool_handler: Option<&mut dyn EvaluatorDynamicToolHandler>,
    ) -> Result<String, EvaluatorError> {
        let request = turn_start_request(
            session_id,
            prompt,
            model,
            thinking,
            output_schema,
            self.session_cwd(session_id),
            self.no_sandbox(),
        )?;
        let response = self.send_turn_request(
            "turn/start",
            AppServerTurnRequest::new(session_id, request),
            dynamic_tool_handler,
        )?;
        Ok(response)
    }

    fn take_last_turn_usage(&mut self) -> Option<EvaluatorTurnUsage> {
        self.take_last_turn_usage_record()
    }

    fn take_retired_sessions(&mut self) -> Vec<String> {
        self.drain_retired_sessions()
    }

    fn set_progress_reporter(&mut self, progress: Option<EvaluatorProgress>) {
        AppServerRunner::set_progress_reporter(self, progress);
    }
}

pub(crate) fn turn_start_request(
    session_id: &str,
    prompt: &str,
    model: Option<&str>,
    thinking: &str,
    output_schema: &Value,
    cwd: Option<&Path>,
    no_sandbox: bool,
) -> Result<Value, EvaluatorError> {
    // Enforce the Interrogation Policy response schema selected by this
    // interrogation's q-scope. For q-scope ["."], the schema excludes
    // ScopeTooNarrow before the evaluator turn is started. This is the
    // one-turn request boundary; retry and q-scope verification orchestration
    // lives in `src/check/run/execute/expectation.rs` for check runs and
    // `src/check/interrogation/query/mod.rs` for `canon ask`. The shared
    // q-scope verification gate lives in `src/check/interrogation/policy.rs`.
    let mut request = json!({
        "threadId": session_id,
        "input": [
            {
                "type": "text",
                "text": prompt
            }
        ],
        "outputSchema": output_schema.clone()
    });
    if let Some(cwd) = cwd {
        request["cwd"] = Value::String(path_to_json_string(cwd, "turn/start cwd")?);
        request["environments"] = json!([local_environment_params(cwd)?]);
    }
    request["sandboxPolicy"] = turn_sandbox_policy(no_sandbox);
    if let Some(model) = model {
        request["model"] = Value::String(model.to_string());
    }
    if let Some(effort) = codex_reasoning_effort(thinking) {
        request["effort"] = Value::String(effort.to_string());
    }
    Ok(request)
}

fn thread_start_sandbox_mode(no_sandbox: bool) -> &'static str {
    if no_sandbox {
        "danger-full-access"
    } else {
        "read-only"
    }
}

fn turn_sandbox_policy(no_sandbox: bool) -> Value {
    if no_sandbox {
        json!({ "type": "dangerFullAccess" })
    } else {
        // This runtime guard is what makes evaluator filesystem write attempts
        // fail without producing observable state changes.
        json!({ "type": "readOnly", "networkAccess": false })
    }
}

#[derive(Serialize)]
struct ThreadStartParams<'a> {
    cwd: String,
    #[serde(rename = "baseInstructions")]
    base_instructions: &'a str,
    #[serde(rename = "developerInstructions")]
    developer_instructions: &'a str,
    #[serde(rename = "approvalPolicy")]
    approval_policy: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    sandbox: Option<&'a str>,
    environments: Vec<TurnEnvironmentParams>,
    config: Value,
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
struct TurnEnvironmentParams {
    #[serde(rename = "environmentId")]
    environment_id: &'static str,
    cwd: String,
}

fn local_environment_params(cwd: &Path) -> Result<TurnEnvironmentParams, EvaluatorError> {
    Ok(TurnEnvironmentParams {
        environment_id: LOCAL_ENVIRONMENT_ID,
        cwd: path_to_json_string(cwd, "local environment cwd")?,
    })
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
