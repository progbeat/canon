use crate::app_server::AppServerRunner;
use crate::app_server_protocol::{context_compaction_event, token_usage_update};
use crate::evaluator_types::EvaluatorError;
use crate::token_usage_types::{
    ContextCompactionEvent, EvaluatorTurnUsage, TokenUsage, TokenUsageUpdate,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

const SESSION_RETIRE_CARRYOVER_TOKEN_LIMIT: u64 = 50_000;

impl AppServerRunner {
    pub(crate) fn turn_usage_for_turn(&self, thread_id: &str, turn_id: &str) -> EvaluatorTurnUsage {
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

    pub(crate) fn apply_thread_reuse_policy(&mut self, turn_usage: &EvaluatorTurnUsage) {
        // App-server token accounting owns local session retirement.
        // `check_interrogation.rs` owns the human-readable thread lifecycle log
        // events that expose effective base and developer instructions.
        if thread_reuse_policy_should_retire(carryover_tokens(turn_usage.usage)) {
            self.retired_sessions.insert(turn_usage.thread_id.clone());
        }
    }

    pub(crate) fn record_app_server_events(&mut self, message: &Value) {
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
