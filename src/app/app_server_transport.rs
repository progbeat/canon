use crate::app_server::AppServerRunner;
use crate::app_server_protocol::{
    agent_message_delta, app_server_error_value, app_server_failure_from_value, app_server_message,
    append_completed_agent_text, turn_idle_timed_out, turn_started_id, turn_text,
};
use crate::evaluator_turn::EvaluatorFailureKind;
use crate::evaluator_types::EvaluatorError;
use crate::platform::check_interrupted;
use crate::APP_SERVER_TURN_TIMEOUT_SECS;
use serde_json::Value;
use std::time::Instant;

pub(crate) struct AppServerTurnRequest {
    thread_id: String,
    params: Value,
}

impl AppServerTurnRequest {
    pub(crate) fn new(thread_id: impl Into<String>, params: Value) -> AppServerTurnRequest {
        AppServerTurnRequest {
            thread_id: thread_id.into(),
            params,
        }
    }
}

impl AppServerRunner {
    pub(crate) fn send_request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Value, EvaluatorError> {
        let id = self.send_json_rpc_request(method, &params, "request")?;
        let mut last_activity = Instant::now();
        loop {
            if check_interrupted() {
                return Err("interrupted".into());
            }
            let Some(message) = self.read_message_or_timeout()? else {
                if turn_idle_timed_out(last_activity, Instant::now()) {
                    return Err(EvaluatorError::failure(
                        EvaluatorFailureKind::TurnTimeout,
                        format!(
                            "app-server {} timed out after {} seconds without response",
                            method, APP_SERVER_TURN_TIMEOUT_SECS
                        ),
                    ));
                }
                continue;
            };
            last_activity = Instant::now();
            self.record_app_server_events(&message);
            let envelope = app_server_message(&message).map_err(app_server_protocol_error)?;
            if envelope.id == Some(id) {
                if let Some(error) = envelope.error.as_ref() {
                    return Err(app_server_failure_from_value(method, error));
                }
                return envelope.result.ok_or_else(|| {
                    EvaluatorError::message(format!(
                        "app-server {} response missing result",
                        method
                    ))
                });
            }
        }
    }

    pub(crate) fn send_turn_request(
        &mut self,
        method: &str,
        request: AppServerTurnRequest,
    ) -> Result<String, EvaluatorError> {
        self.last_turn_usage = None;
        let id = self.send_json_rpc_request(method, &request.params, "request")?;
        let thread_id = request.thread_id;

        let mut saw_response = false;
        let mut saw_completed = false;
        let mut text = String::new();
        let mut completed_text = String::new();
        let mut last_activity = Instant::now();
        let mut turn_id: Option<String> = None;
        let mut pending_error: Option<Value> = None;
        let mut interrupted = false;
        let mut interrupt_sent = false;
        loop {
            self.maybe_interrupt_turn(
                &mut interrupted,
                &mut interrupt_sent,
                Some(thread_id.as_str()),
                turn_id.as_deref(),
            )?;
            let Some(message) = self.read_message_or_timeout()? else {
                if turn_idle_timed_out(last_activity, Instant::now()) {
                    return Err(EvaluatorError::failure(
                        EvaluatorFailureKind::TurnTimeout,
                        format!(
                            "app-server {} timed out after {} seconds without progress",
                            method, APP_SERVER_TURN_TIMEOUT_SECS
                        ),
                    ));
                }
                continue;
            };
            last_activity = Instant::now();
            self.record_app_server_events(&message);
            let envelope = app_server_message(&message).map_err(app_server_protocol_error)?;
            if let Some(started_turn_id) = turn_started_id(&message) {
                turn_id = Some(started_turn_id);
                self.maybe_interrupt_turn(
                    &mut interrupted,
                    &mut interrupt_sent,
                    Some(thread_id.as_str()),
                    turn_id.as_deref(),
                )?;
            }
            if envelope.id == Some(id) {
                if let Some(error) = envelope
                    .error
                    .as_ref()
                    .cloned()
                    .or_else(|| pending_error.take())
                {
                    return Err(self.fail_turn_request(
                        method,
                        &error,
                        &thread_id,
                        turn_id.as_deref(),
                    ));
                }
                saw_response = true;
                if saw_completed {
                    return self.finish_turn_request(text, completed_text, &thread_id, turn_id);
                }
                continue;
            }
            match envelope.method.as_deref() {
                Some("item/agentMessage/delta") => {
                    if let Some(delta) = agent_message_delta(&message) {
                        text.push_str(&delta);
                    }
                }
                Some("item/completed") | Some("item/agentMessage/completed") => {
                    append_completed_agent_text(&message, &mut completed_text);
                }
                Some("turn/completed") => {
                    if interrupted {
                        return Err("interrupted".into());
                    }
                    if let Some(error) =
                        app_server_error_value(&message).or_else(|| pending_error.take())
                    {
                        return Err(self.fail_turn_request(
                            method,
                            &error,
                            &thread_id,
                            turn_id.as_deref(),
                        ));
                    }
                    saw_completed = true;
                    if saw_response {
                        return self.finish_turn_request(text, completed_text, &thread_id, turn_id);
                    }
                }
                Some("error") => {
                    if let Some(error) = app_server_error_value(&message) {
                        pending_error = Some(error);
                    }
                }
                Some(_) => {
                    if let Some(error) = app_server_error_value(&message) {
                        return Err(self.fail_turn_request(
                            method,
                            &error,
                            &thread_id,
                            turn_id.as_deref(),
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    fn finish_turn_request(
        &mut self,
        text: String,
        completed_text: String,
        thread_id: &str,
        turn_id: Option<String>,
    ) -> Result<String, EvaluatorError> {
        self.drain_token_usage_updates()?;
        let completed_turn_usage = turn_id
            .as_deref()
            .map(|turn_id| self.turn_usage_for_turn(thread_id, turn_id));
        if let Some(turn_usage) = completed_turn_usage {
            self.apply_thread_reuse_policy(&turn_usage);
            self.last_turn_usage = Some(turn_usage);
        }
        Ok(turn_text(text, completed_text))
    }

    fn fail_turn_request(
        &mut self,
        method: &str,
        error: &Value,
        thread_id: &str,
        turn_id: Option<&str>,
    ) -> EvaluatorError {
        if let Err(err) = self.drain_token_usage_updates() {
            return err;
        }
        self.last_turn_usage = turn_id.map(|turn_id| self.turn_usage_for_turn(thread_id, turn_id));
        app_server_failure_from_value(method, error)
    }
}

fn app_server_protocol_error(error: String) -> EvaluatorError {
    EvaluatorError::failure(EvaluatorFailureKind::UnknownAppServer, error)
}
