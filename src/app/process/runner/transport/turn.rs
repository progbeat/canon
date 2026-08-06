use super::super::AppServerRunner;
use crate::app::protocol::{
    app_server_error_value, app_server_failure_from_value, app_server_message, turn_started_id,
    turn_text, AppServerEventKind,
};
use crate::evaluator::{EvaluatorDynamicToolHandler, EvaluatorError, EvaluatorFailureKind};
use serde_json::Value;
use std::time::Instant;

mod completion;
mod dynamic_tool;
mod progress;

use super::timeout::{idle_timeout_elapsed, idle_timeout_error, IdleTimeoutKind};
use dynamic_tool::handle_dynamic_tool_call;
use progress::ActiveTurnProgress;

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
    pub(crate) fn send_turn_request(
        &mut self,
        method: &str,
        request: AppServerTurnRequest,
        mut dynamic_tool_handler: Option<&mut dyn EvaluatorDynamicToolHandler>,
    ) -> Result<String, EvaluatorError> {
        self.last_turn_usage = None;
        let id = self.send_json_rpc_request(method, &request.params, "request")?;
        let active_turn_progress = ActiveTurnProgress::start(self.progress.clone());
        let thread_id = request.thread_id;

        let mut saw_response = false;
        let mut saw_completed = false;
        let mut agent_message_delta_text = String::new();
        let mut agent_message_completed_text = String::new();
        let mut last_activity = Instant::now();
        let mut turn_id: Option<String> = None;
        // [w] Every exit after the app server assigns a turn ID converges on
        // `finish_turn_usage` below, including protocol, timeout, interruption,
        // and app-server error paths.
        let result = (|| {
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
                let message = self.read_message_or_timeout()?;
                let Some(message) = message else {
                    let now = Instant::now();
                    if idle_timeout_elapsed(last_activity, now) {
                        // A turn attempt is active here, so an exhausted
                        // no-progress timeout owns the timeline `×` marker.
                        self.record_turn_timeout_progress();
                        return Err(idle_timeout_error(method, IdleTimeoutKind::Progress));
                    }
                    continue;
                };
                last_activity = Instant::now();
                self.record_turn_message_activity_progress();
                let mut envelope = app_server_message(&message).map_err(|error| {
                    EvaluatorError::failure(EvaluatorFailureKind::UnknownAppServer, error)
                })?;
                self.record_app_server_events(&envelope)?;
                if let Some(started_turn_id) = turn_started_id(&envelope) {
                    turn_id = Some(started_turn_id);
                    self.maybe_interrupt_turn(
                        &mut interrupted,
                        &mut interrupt_sent,
                        Some(thread_id.as_str()),
                        turn_id.as_deref(),
                    )?;
                }
                if envelope.response_id == Some(id) {
                    Self::ensure_no_turn_error(
                        method,
                        envelope.error.cloned().or_else(|| pending_error.take()),
                    )?;
                    saw_response = true;
                    if saw_completed {
                        break;
                    }
                    continue;
                }
                match envelope.kind {
                    AppServerEventKind::DynamicToolCall => {
                        let Some(tool_call_id) = envelope.request_id else {
                            return Err(EvaluatorError::failure(
                                EvaluatorFailureKind::UnknownAppServer,
                                "app-server dynamic tool call missing id",
                            ));
                        };
                        let response =
                            handle_dynamic_tool_call(&mut envelope, &mut dynamic_tool_handler);
                        self.send_json_rpc_response(
                            tool_call_id,
                            &response,
                            "dynamic tool response",
                        )?;
                    }
                    AppServerEventKind::AgentMessageDelta => {
                        if let Some(message_text) = envelope.agent_message_delta_text {
                            agent_message_delta_text.push_str(message_text);
                        }
                    }
                    AppServerEventKind::ItemCompleted
                    | AppServerEventKind::AgentMessageCompleted => {
                        if let Some(message_text) = envelope.agent_message_completed_text {
                            agent_message_completed_text = message_text;
                        }
                    }
                    AppServerEventKind::TurnCompleted => {
                        if interrupted {
                            return Err(EvaluatorError::interrupted());
                        }
                        Self::ensure_no_turn_error(
                            method,
                            app_server_error_value(&envelope).or_else(|| pending_error.take()),
                        )?;
                        saw_completed = true;
                        if saw_response {
                            break;
                        }
                    }
                    AppServerEventKind::Error => {
                        if let Some(error) = app_server_error_value(&envelope) {
                            pending_error = Some(error);
                        }
                    }
                    _ => {
                        Self::ensure_no_turn_error(method, app_server_error_value(&envelope))?;
                    }
                }
            }
            Ok(turn_text(
                agent_message_delta_text,
                agent_message_completed_text,
            ))
        })();
        // [2gZ,w] The turn attempt ends with its transport result. Usage
        // cleanup still runs for every exit, but must not extend the active
        // no-progress interval or produce a spurious `~` minute.
        drop(active_turn_progress);
        let usage_result = self.finish_turn_usage(&thread_id, turn_id.as_deref());
        match (result, usage_result) {
            (Ok(response), Ok(())) => Ok(response),
            (Ok(_), Err(usage_error)) => Err(usage_error),
            (Err(error), Ok(())) => Err(error),
            (Err(error), Err(usage_error)) => Err(error.with_appended_message(format!(
                "failed to drain all evaluator turn usage updates: {usage_error}"
            ))),
        }
    }

    fn ensure_no_turn_error(method: &str, error: Option<Value>) -> Result<(), EvaluatorError> {
        let Some(error) = error else {
            return Ok(());
        };
        Err(app_server_failure_from_value(method, &error))
    }
}
