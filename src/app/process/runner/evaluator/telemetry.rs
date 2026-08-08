//! Collects runner telemetry and attributes cumulative notifications to turns.

use crate::app::protocol::{
    app_server_message, context_compaction_event, token_usage_update, AppServerMessage,
    UnsequencedContextCompactionEvent, UnsequencedTokenUsageUpdate,
};
use crate::evaluator::{EvaluatorError, EvaluatorFailureKind};
use crate::token_usage::{
    ContextCompactionEvent, EvaluatorTurnUsage, TokenUsage, TokenUsageUpdate,
};
use std::collections::BTreeMap;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use super::super::AppServerRunner;

const THREAD_RETIRE_CARRYOVER_TOKEN_LIMIT: u64 = 50_000;

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
        // The runner owns local thread retirement based on app-server carryover usage.
        // Evaluator interrogation lifecycle logging owns the human-readable
        // events that expose effective base and developer instructions.
        if thread_reuse_policy_should_retire(carryover_tokens(turn_usage.usage)) {
            self.retired_threads.insert(turn_usage.thread_id.clone());
        }
    }

    pub(crate) fn record_app_server_events(
        &mut self,
        message: &AppServerMessage<'_>,
    ) -> Result<(), EvaluatorError> {
        if let Some(update) = token_usage_update(message) {
            record_token_usage_update(
                &mut self.token_usage_by_turn,
                &mut self.latest_token_usage_by_thread,
                &mut self.token_usage_updates_by_turn,
                update,
            )?;
        }
        if let Some(event) = context_compaction_event(message) {
            record_context_compaction_event(&mut self.context_compaction_events_by_turn, event);
        }
        Ok(())
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
                Ok(Ok(message)) => {
                    self.record_turn_message_activity_progress();
                    let message = app_server_message(&message).map_err(|error| {
                        EvaluatorError::failure(EvaluatorFailureKind::UnknownAppServer, error)
                    })?;
                    self.record_app_server_events(&message)?;
                }
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
    event: UnsequencedContextCompactionEvent,
) {
    let events = context_compaction_events_by_turn
        .entry(event.turn_id.clone())
        .or_default();
    events.push(ContextCompactionEvent {
        sequence: events.len() as u64 + 1,
        thread_id: event.thread_id,
        turn_id: event.turn_id,
        method: event.method,
        event: event.event,
    });
}

pub(crate) fn record_token_usage_update(
    token_usage_by_turn: &mut BTreeMap<String, TokenUsage>,
    latest_token_usage_by_thread: &mut BTreeMap<String, TokenUsage>,
    token_usage_updates_by_turn: &mut BTreeMap<String, Vec<TokenUsageUpdate>>,
    update: UnsequencedTokenUsageUpdate,
) -> Result<(), EvaluatorError> {
    let thread_id = update.thread_id.clone();
    let turn_id = update.turn_id.clone();
    let previous_thread_total = latest_token_usage_by_thread
        .get(&thread_id)
        .copied()
        .unwrap_or_default();
    // App-server usage notifications are cumulative thread snapshots and may
    // repeat unchanged. Only their delta belongs to this notification's turn.
    let usage = checked_token_usage_delta(update.thread_total_usage, previous_thread_total)
        .ok_or_else(|| {
            EvaluatorError::failure(
                EvaluatorFailureKind::UnknownAppServer,
                format!(
                    "app-server cumulative token usage decreased for thread {thread_id}: \
                     previous={previous_thread_total:?}, current={:?}",
                    update.thread_total_usage
                ),
            )
        })?;
    let updates = token_usage_updates_by_turn
        .entry(turn_id.clone())
        .or_default();
    let thread_total_usage = update.thread_total_usage;
    updates.push(TokenUsageUpdate {
        sequence: updates.len() as u64 + 1,
        thread_id: update.thread_id,
        turn_id: update.turn_id,
        token_usage: update.token_usage,
        thread_total_usage,
    });
    latest_token_usage_by_thread.insert(thread_id, thread_total_usage);
    let current = token_usage_by_turn
        .get(&turn_id)
        .copied()
        .unwrap_or_default();
    token_usage_by_turn.insert(turn_id, current.add(usage));
    Ok(())
}

fn checked_token_usage_delta(current: TokenUsage, previous: TokenUsage) -> Option<TokenUsage> {
    Some(TokenUsage {
        total_tokens: current.total_tokens.checked_sub(previous.total_tokens)?,
        input_tokens: current.input_tokens.checked_sub(previous.input_tokens)?,
        cached_input_tokens: current
            .cached_input_tokens
            .checked_sub(previous.cached_input_tokens)?,
        output_tokens: current.output_tokens.checked_sub(previous.output_tokens)?,
        reasoning_output_tokens: current
            .reasoning_output_tokens
            .checked_sub(previous.reasoning_output_tokens)?,
    })
}

pub(crate) fn carryover_tokens(usage: TokenUsage) -> u64 {
    usage.input_tokens + usage.output_tokens
}

pub(crate) fn thread_reuse_policy_should_retire(current_carryover_tokens: u64) -> bool {
    current_carryover_tokens > THREAD_RETIRE_CARRYOVER_TOKEN_LIMIT
}

#[cfg(test)]
mod tests {
    use super::record_token_usage_update;
    use crate::app::protocol::UnsequencedTokenUsageUpdate;
    use crate::evaluator::EvaluatorFailureKind;
    use crate::token_usage::TokenUsage;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test] // xpec: gN
    fn cumulative_thread_snapshots_become_exact_per_turn_usage() {
        let mut token_usage_by_turn = BTreeMap::new();
        let mut latest_token_usage_by_thread = BTreeMap::new();
        let mut token_usage_updates_by_turn = BTreeMap::new();

        record_token_usage_update(
            &mut token_usage_by_turn,
            &mut latest_token_usage_by_thread,
            &mut token_usage_updates_by_turn,
            update("turn-1", usage(10, 8, 2)),
        )
        .unwrap();
        record_token_usage_update(
            &mut token_usage_by_turn,
            &mut latest_token_usage_by_thread,
            &mut token_usage_updates_by_turn,
            update("turn-1", usage(10, 8, 2)),
        )
        .unwrap();
        record_token_usage_update(
            &mut token_usage_by_turn,
            &mut latest_token_usage_by_thread,
            &mut token_usage_updates_by_turn,
            update("turn-2", usage(25, 20, 5)),
        )
        .unwrap();

        assert_eq!(token_usage_by_turn["turn-1"], usage(10, 8, 2));
        assert_eq!(token_usage_by_turn["turn-2"], usage(15, 12, 3));
        assert_eq!(token_usage_updates_by_turn["turn-1"].len(), 2);
    }

    #[test] // xpec: gO
    fn decreasing_cumulative_snapshot_is_rejected_atomically() {
        let mut token_usage_by_turn = BTreeMap::new();
        let mut latest_token_usage_by_thread = BTreeMap::new();
        let mut token_usage_updates_by_turn = BTreeMap::new();
        record_token_usage_update(
            &mut token_usage_by_turn,
            &mut latest_token_usage_by_thread,
            &mut token_usage_updates_by_turn,
            update("turn-1", usage(10, 8, 2)),
        )
        .unwrap();
        let state_before = (
            token_usage_by_turn.clone(),
            latest_token_usage_by_thread.clone(),
            token_usage_updates_by_turn.clone(),
        );

        let error = record_token_usage_update(
            &mut token_usage_by_turn,
            &mut latest_token_usage_by_thread,
            &mut token_usage_updates_by_turn,
            update("turn-2", usage(20, 7, 13)),
        )
        .unwrap_err();

        assert_eq!(error.kind(), Some(EvaluatorFailureKind::UnknownAppServer));
        assert!(error
            .message_str()
            .contains("cumulative token usage decreased"));
        assert_eq!(
            (
                token_usage_by_turn,
                latest_token_usage_by_thread,
                token_usage_updates_by_turn,
            ),
            state_before
        );
    }

    fn update(turn_id: &str, thread_total_usage: TokenUsage) -> UnsequencedTokenUsageUpdate {
        UnsequencedTokenUsageUpdate {
            thread_id: "thread".to_string(),
            turn_id: turn_id.to_string(),
            token_usage: json!({}),
            thread_total_usage,
        }
    }

    fn usage(total_tokens: u64, input_tokens: u64, output_tokens: u64) -> TokenUsage {
        TokenUsage {
            total_tokens,
            input_tokens,
            cached_input_tokens: 0,
            output_tokens,
            reasoning_output_tokens: 0,
        }
    }
}
