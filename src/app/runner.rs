use crate::app::server::{AppServerRunner, LazyAppServerRunner};
use crate::app::transport::AppServerTurnRequest;
use crate::check::validation::codex_reasoning_effort;
use crate::config_types::AgentConfig;
use crate::evaluator::config::evaluator_thread_config_with_no_sandbox;
use crate::evaluator::prompt::EVALUATOR_BASE_INSTRUCTIONS;
use crate::evaluator::turn::is_model_technical_failure;
use crate::evaluator::types::{EvaluatorError, EvaluatorRunner};
use crate::evaluator::{evaluator_turn_input, render_evaluator_turn_input};
use crate::token_usage_types::{EvaluatorTurnUsage, TokenUsage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const EVALUATOR_SESSION_START_SOURCE: &str = "clear";
const LOCAL_ENVIRONMENT_ID: &str = "local";

impl LazyAppServerRunner {
    pub(crate) fn token_usage(&self) -> Option<TokenUsage> {
        let mut total = self.retired_token_usage;
        if let Some(usage) = self.inner.as_ref().and_then(AppServerRunner::token_usage) {
            total = total.add(usage);
        }
        if total.total_tokens == 0 {
            None
        } else {
            Some(total)
        }
    }

    pub(crate) fn drain_token_usage_updates(&mut self) -> Result<(), EvaluatorError> {
        if let Some(inner) = self.inner.as_mut() {
            inner.drain_token_usage_updates()?;
        }
        Ok(())
    }

    fn retire_inner_after_model_failure(
        &mut self,
        err: &EvaluatorError,
    ) -> Result<(), EvaluatorError> {
        if !is_model_technical_failure(err) {
            return Ok(());
        }
        if let Some(inner) = self.inner.as_mut() {
            inner.drain_token_usage_updates()?;
            if let Some(usage) = inner.token_usage() {
                self.retired_token_usage = self.retired_token_usage.add(usage);
            }
        }
        self.sessions.clear();
        self.inner = None;
        Ok(())
    }
}

impl EvaluatorRunner for LazyAppServerRunner {
    fn start_session(
        &mut self,
        session_cwd: &Path,
        instructions: &str,
        agent: &AgentConfig,
        model: Option<&str>,
        thinking: &str,
        scope: &[String],
    ) -> Result<String, EvaluatorError> {
        let result =
            self.inner()?
                .start_session(session_cwd, instructions, agent, model, thinking, scope);
        match result {
            Ok(session_id) => {
                self.sessions.insert(session_id.clone());
                Ok(session_id)
            }
            Err(err) => {
                self.retire_inner_after_model_failure(&err)?;
                Err(err)
            }
        }
    }

    fn ask(
        &mut self,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
        thinking: &str,
    ) -> Result<String, EvaluatorError> {
        if !self.sessions.contains(session_id) {
            return Err("app-server runner does not own session".into());
        }
        let result = self
            .inner
            .as_mut()
            .ok_or_else(|| EvaluatorError::message("app-server runner is not initialized"))?
            .ask(session_id, prompt, model, thinking);
        if let Err(err) = &result {
            self.retire_inner_after_model_failure(err)?;
        }
        result
    }

    fn take_last_turn_usage(&mut self) -> Option<EvaluatorTurnUsage> {
        self.inner
            .as_mut()
            .and_then(AppServerRunner::take_last_turn_usage)
    }

    fn take_retired_sessions(&mut self) -> Vec<String> {
        let Some(inner) = self.inner.as_mut() else {
            return Vec::new();
        };
        let retired = inner.drain_retired_sessions();
        for session_id in &retired {
            self.sessions.remove(session_id);
        }
        retired
    }
}

impl EvaluatorRunner for AppServerRunner {
    fn start_session(
        &mut self,
        session_cwd: &Path,
        instructions: &str,
        agent: &AgentConfig,
        model: Option<&str>,
        thinking: &str,
        scope: &[String],
    ) -> Result<String, EvaluatorError> {
        // `session_cwd` is the staged Git snapshot root supplied by
        // `check_interrogation`; it is distinct from `LazyAppServerRunner`'s
        // app-server startup root, which is the real project root used for
        // Canon runtime state and app-server configuration.
        let params = ThreadStartParams {
            cwd: session_cwd.display().to_string(),
            base_instructions: EVALUATOR_BASE_INSTRUCTIONS,
            developer_instructions: instructions,
            approval_policy: "never",
            sandbox: self.no_sandbox.then_some("danger-full-access"),
            environments: vec![local_environment_params(session_cwd)],
            config: evaluator_thread_config_with_no_sandbox(
                agent,
                scope,
                model,
                thinking,
                session_cwd,
                self.no_sandbox,
            ),
            // Evaluator threads are invocation-local and ephemeral. Canon still
            // reuses live thread IDs by scope within this `canon check`, but
            // oversized carryover is handled by retiring the local session ID
            // so the next same-scope interrogation starts a fresh thread.
            ephemeral: true,
            // Evaluator threads must not inherit the parent Codex conversation:
            // canon questions about "your dev instructions" refer only to the
            // rendered evaluator developerInstructions parameter below.
            session_start_source: EVALUATOR_SESSION_START_SOURCE,
        };
        let params = serde_json::to_value(params)
            .map_err(|err| format!("failed to encode thread/start params: {}", err))?;
        let result = self.send_request("thread/start", params)?;
        let response: ThreadStartResponse = serde_json::from_value(result)
            .map_err(|err| format!("thread/start response missing thread.id: {}", err))?;
        self.session_cwds
            .insert(response.thread.id.clone(), session_cwd.to_path_buf());
        Ok(response.thread.id)
    }

    fn ask(
        &mut self,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
        thinking: &str,
    ) -> Result<String, EvaluatorError> {
        let request = turn_start_request(
            session_id,
            prompt,
            model,
            thinking,
            self.session_cwds.get(session_id).map(PathBuf::as_path),
            self.no_sandbox,
        )?;
        let response =
            self.send_turn_request("turn/start", AppServerTurnRequest::new(session_id, request))?;
        Ok(response)
    }

    fn take_last_turn_usage(&mut self) -> Option<EvaluatorTurnUsage> {
        self.last_turn_usage.take()
    }

    fn take_retired_sessions(&mut self) -> Vec<String> {
        self.drain_retired_sessions()
    }
}

pub(crate) fn turn_start_request(
    session_id: &str,
    prompt: &str,
    model: Option<&str>,
    thinking: &str,
    cwd: Option<&Path>,
    no_sandbox: bool,
) -> Result<Value, EvaluatorError> {
    let input = evaluator_turn_input(prompt)?;
    let input_text = render_evaluator_turn_input(&input)?;
    let mut request = json!({
        "threadId": session_id,
        "input": [
            {
                "type": "text",
                "text": input_text
            }
        ]
    });
    if let Some(cwd) = cwd {
        request["cwd"] = Value::String(cwd.display().to_string());
        request["environments"] = json!([local_environment_params(cwd)]);
    }
    if no_sandbox {
        request["sandboxPolicy"] = json!({ "type": "dangerFullAccess" });
    }
    if let Some(model) = model {
        request["model"] = Value::String(model.to_string());
    }
    if let Some(effort) = codex_reasoning_effort(thinking) {
        request["effort"] = Value::String(effort.to_string());
    }
    Ok(request)
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
    #[serde(rename = "sessionStartSource")]
    session_start_source: &'a str,
}

#[derive(Serialize)]
struct TurnEnvironmentParams {
    #[serde(rename = "environmentId")]
    environment_id: &'static str,
    cwd: String,
}

fn local_environment_params(cwd: &Path) -> TurnEnvironmentParams {
    TurnEnvironmentParams {
        environment_id: LOCAL_ENVIRONMENT_ID,
        cwd: cwd.display().to_string(),
    }
}

#[derive(Deserialize)]
struct ThreadStartResponse {
    thread: ThreadStartThread,
}

#[derive(Deserialize)]
struct ThreadStartThread {
    id: String,
}
