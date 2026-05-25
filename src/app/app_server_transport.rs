use crate::app_server::AppServerRunner;
use crate::app_server_protocol::{
    agent_message_delta, app_server_error_value, app_server_failure_from_value, app_server_message,
    append_completed_agent_text, context_compaction_event, token_usage_update, turn_idle_timed_out,
    turn_started_id, turn_text,
};
use crate::check_preflight::check_interrupted;
use crate::evaluator_turn::EvaluatorFailureKind;
use crate::evaluator_types::EvaluatorError;
use crate::token_usage_types::{
    ContextCompactionEvent, EvaluatorTurnUsage, TokenUsage, TokenUsageUpdate,
};
use crate::APP_SERVER_TURN_TIMEOUT_SECS;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

const SESSION_RETIRE_CARRYOVER_TOKEN_LIMIT: u64 = 50_000;

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

    fn turn_usage_for_turn(&self, thread_id: &str, turn_id: &str) -> EvaluatorTurnUsage {
        let usage = self
            .token_usage_by_turn
            .get(turn_id)
            .copied()
            .unwrap_or_default();
        let updates = self
            .token_usage_updates_by_turn
            .get(turn_id)
            .cloned()
            .unwrap_or_default();
        let compaction_events = self
            .context_compaction_events_by_turn
            .get(turn_id)
            .cloned()
            .unwrap_or_default();
        EvaluatorTurnUsage {
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            usage,
            token_usage_updates: updates,
            context_compaction_events: compaction_events,
        }
    }

    fn apply_thread_reuse_policy(&mut self, turn_usage: &EvaluatorTurnUsage) {
        // This transport owns app-server token accounting and local session
        // retirement. `check_interrogation.rs` owns the human-readable thread
        // lifecycle log events (`thread.start`/`thread.reuse`) that expose the
        // effective base and developer instructions for each evaluator thread.
        if thread_reuse_policy_should_retire(carryover_tokens(turn_usage.usage)) {
            self.retired_sessions.insert(turn_usage.thread_id.clone());
        }
    }

    fn maybe_interrupt_turn(
        &mut self,
        interrupted: &mut bool,
        interrupt_sent: &mut bool,
        thread_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Result<(), EvaluatorError> {
        if !check_interrupted() {
            return Ok(());
        }
        *interrupted = true;
        if *interrupt_sent {
            return Ok(());
        }
        let (Some(thread_id), Some(turn_id)) = (thread_id, turn_id) else {
            return Ok(());
        };
        self.send_turn_interrupt(thread_id, turn_id)?;
        *interrupt_sent = true;
        Ok(())
    }

    fn send_turn_interrupt(
        &mut self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), EvaluatorError> {
        let params = json!({
            "threadId": thread_id,
            "turnId": turn_id
        });
        self.send_json_rpc_request("turn/interrupt", &params, "interrupt")?;
        Ok(())
    }

    fn send_json_rpc_request(
        &mut self,
        method: &str,
        params: &Value,
        operation: &str,
    ) -> Result<u64, EvaluatorError> {
        if check_interrupted() {
            return Err("interrupted".into());
        }
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        writeln!(self.stdin, "{}", request)
            .map_err(|err| format!("failed to write app-server {}: {}", operation, err))?;
        self.stdin
            .flush()
            .map_err(|err| format!("failed to flush app-server {}: {}", operation, err))?;
        Ok(id)
    }

    fn read_message_or_timeout(&mut self) -> Result<Option<Value>, EvaluatorError> {
        match self.messages.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => result.map(Some).map_err(EvaluatorError::message),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                if check_interrupted() {
                    Err("interrupted".into())
                } else {
                    Err(self.app_server_closed_stdout_error())
                }
            }
        }
    }

    fn app_server_closed_stdout_error(&mut self) -> EvaluatorError {
        let mut message = String::from("app-server closed stdout");
        if let Ok(Some(status)) = self.child.try_wait() {
            message.push_str(&format!(" with status {}", status));
        }
        let stderr = self
            .stderr
            .try_iter()
            .collect::<Vec<_>>()
            .join("")
            .trim()
            .to_string();
        if !stderr.is_empty() {
            message.push_str(": ");
            message.push_str(&stderr);
        }
        EvaluatorError::message(message)
    }

    fn record_app_server_events(&mut self, message: &Value) {
        if let Some(update) = token_usage_update(message) {
            record_token_usage_update(
                &mut self.token_usage_by_turn,
                &mut self.token_usage_updates_by_turn,
                update,
            );
        }
        if let Some(event) = context_compaction_event(message) {
            record_context_compaction_event(&mut self.context_compaction_events_by_turn, event);
        }
    }

    pub(crate) fn token_usage(&self) -> Option<TokenUsage> {
        let mut usage = TokenUsage::default();
        for turn_usage in self.token_usage_by_turn.values() {
            usage = usage.add(*turn_usage);
        }
        if usage.total_tokens == 0 {
            None
        } else {
            Some(usage)
        }
    }

    pub(crate) fn drain_token_usage_updates(&mut self) -> Result<(), EvaluatorError> {
        loop {
            match self.messages.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(message)) => self.record_app_server_events(&message),
                Ok(Err(err)) => return Err(EvaluatorError::message(err)),
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
                    return Ok(());
                }
            }
        }
    }
}

pub(crate) fn record_context_compaction_event(
    context_compaction_events_by_turn: &mut BTreeMap<String, Vec<ContextCompactionEvent>>,
    mut event: ContextCompactionEvent,
) {
    let events = context_compaction_events_by_turn
        .entry(event.turn_id.clone())
        .or_default();
    event.sequence = events.len() as u64 + 1;
    events.push(event);
}

pub(crate) fn record_token_usage_update(
    token_usage_by_turn: &mut BTreeMap<String, TokenUsage>,
    token_usage_updates_by_turn: &mut BTreeMap<String, Vec<TokenUsageUpdate>>,
    mut update: TokenUsageUpdate,
) {
    let usage = update.last_usage;
    let turn_id = update.turn_id.clone();
    let updates = token_usage_updates_by_turn
        .entry(turn_id.clone())
        .or_default();
    update.sequence = updates.len() as u64 + 1;
    updates.push(update);
    let current = token_usage_by_turn
        .get(&turn_id)
        .copied()
        .unwrap_or_default();
    token_usage_by_turn.insert(turn_id, current.add(usage));
}

pub(crate) fn carryover_tokens(usage: TokenUsage) -> u64 {
    usage.input_tokens + usage.output_tokens
}

pub(crate) fn thread_reuse_policy_should_retire(current_carryover_tokens: u64) -> bool {
    current_carryover_tokens > SESSION_RETIRE_CARRYOVER_TOKEN_LIMIT
}

fn app_server_protocol_error(error: String) -> EvaluatorError {
    EvaluatorError::failure(EvaluatorFailureKind::UnknownAppServer, error)
}
