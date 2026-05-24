use crate::check_types::{
    CheckRecord, CheckRecordOutcome, CheckResult, ObservedAnswerState, ParsedAnswer,
    SelectedExpectation,
};
use crate::config_types::AgentConfig;
use crate::evaluator_prompt::EVALUATOR_BASE_INSTRUCTIONS;
use crate::evaluator_response_cache::{response_excerpt, EvaluatorResponseParseCache};
use crate::evaluator_types::{EvaluatorError, EvaluatorRunner};
use crate::hash::full_scope;
use crate::logging::DiagnosticLogWriter;
use crate::token_usage_types::{EvaluatorTurnUsage, TokenUsage};
use crate::UNPARSEABLE_OBSERVED;
use serde::Serialize;
use serde_json::{json, Value};

pub(crate) fn evaluator_models(agent: &AgentConfig) -> Vec<Option<String>> {
    let mut models = vec![agent.model.primary.clone()];
    models.extend(agent.model.fallbacks.iter().cloned().map(Some));
    models
}

pub(crate) fn effective_thinking<'a>(
    agent: &'a AgentConfig,
    expectation: &'a SelectedExpectation,
) -> &'a str {
    expectation.thinking.as_deref().unwrap_or(&agent.thinking)
}

pub(crate) fn model_label(model: Option<&str>) -> &str {
    model.unwrap_or("<default>")
}

pub(crate) fn is_model_technical_failure(err: &EvaluatorError) -> bool {
    err.kind()
        .is_some_and(EvaluatorFailureKind::is_model_technical)
}

pub(crate) fn is_context_window_failure(err: &EvaluatorError) -> bool {
    err.kind() == Some(EvaluatorFailureKind::ContextWindow)
}

pub(crate) fn session_failure_invalidates_thread(err: &EvaluatorError) -> bool {
    err.kind()
        .is_some_and(EvaluatorFailureKind::invalidates_thread)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EvaluatorFailureKind {
    UsageLimit,
    RateLimit,
    ModelUnavailable,
    TurnTimeout,
    ContextWindow,
    UnknownAppServer,
}

impl EvaluatorFailureKind {
    pub(crate) fn is_model_technical(self) -> bool {
        matches!(
            self,
            EvaluatorFailureKind::UsageLimit
                | EvaluatorFailureKind::RateLimit
                | EvaluatorFailureKind::ModelUnavailable
                | EvaluatorFailureKind::TurnTimeout
                | EvaluatorFailureKind::ContextWindow
                | EvaluatorFailureKind::UnknownAppServer
        )
    }

    pub(crate) fn invalidates_thread(self) -> bool {
        self.is_model_technical()
    }
}

pub(crate) fn record_from_response(
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    response: ParsedAnswer,
    enforced_scope: Vec<String>,
    scope_hash: String,
) -> Result<CheckRecord, String> {
    // This is the expectation-specific answer vocabulary gate: yes/no and
    // option expectations reject prose, while free-form exact-string
    // expectations remain valid single-line answers.
    let requires_human_review =
        ObservedAnswerState::from_expected_and_observed(&expectation.a, &response.answer)
            .requires_human_review();
    let result = if !requires_human_review && response.answer == expectation.a {
        CheckResult::Pass
    } else {
        CheckResult::Fail
    };
    CheckRecord::current_from_expectation(
        agent,
        expectation,
        CheckRecordOutcome {
            result,
            observed: response.answer,
            evidence: response.evidence,
            scope: enforced_scope,
            scope_hash,
        },
    )
}

// One evaluator turn: model labels, response parsing, request and response
// logging, per-turn token usage, and record finalization.
pub(crate) struct EvaluatorTurnContext<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
}

pub(crate) struct ThreadLifecycleLog {
    pub(crate) event: &'static str,
    pub(crate) session_id: String,
    pub(crate) developer_instructions: String,
}

pub(crate) struct ParsedTurnResponse {
    pub(crate) answer: ParsedAnswer,
    pub(crate) usage: Option<TokenUsage>,
    pub(crate) context_compacted: bool,
}

pub(crate) struct RawTurnResponse {
    pub(crate) text: String,
    pub(crate) usage: Option<TokenUsage>,
    pub(crate) context_compacted: bool,
}

pub(crate) fn ask_once<R: EvaluatorRunner>(
    runner: &mut R,
    turn: &EvaluatorTurnContext<'_>,
    prompt: &str,
    agent: &AgentConfig,
    parser_cache: &mut EvaluatorResponseParseCache,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let response = ask_and_log(
        runner,
        turn,
        prompt,
        diagnostic_log,
        expectation_id,
        1,
        "initial",
    )?;
    let parsed = match parser_cache.parse(&response.text, agent) {
        Ok(answer) => answer,
        Err(err) => unparseable_response_answer(&err, &response.text),
    };

    Ok(ParsedTurnResponse {
        answer: parsed,
        usage: response.usage,
        context_compacted: response.context_compacted,
    })
}

fn unparseable_response_answer(err: &str, response: &str) -> ParsedAnswer {
    ParsedAnswer {
        answer: UNPARSEABLE_OBSERVED.to_string(),
        evidence: format!(
            "evaluator response could not be parsed: {}\nresponse: {}",
            err,
            response_excerpt(response)
        ),
        scope: full_scope(),
    }
}

pub(crate) fn ask_and_log<R: EvaluatorRunner>(
    runner: &mut R,
    turn: &EvaluatorTurnContext<'_>,
    prompt: &str,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
) -> Result<RawTurnResponse, EvaluatorError> {
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        let raw_request = serde_json::to_value(EvaluatorTurnLogRequest {
            session_id: turn.session_id,
            prompt,
            model: turn.model,
            thinking: turn.thinking,
        })
        .map_err(|err| format!("failed to encode evaluator turn request log: {}", err))?;
        writer.write_event(
            "info",
            "agent.request",
            &[
                ("id", json!(expectation_id)),
                ("attempt", json!(attempt)),
                ("reason", json!(reason)),
                ("request", raw_request),
            ],
        )?;
    }
    let response = match runner.ask(turn.session_id, prompt, turn.model, turn.thinking) {
        Ok(response) => response,
        Err(err) => {
            let turn_usage = runner.take_last_turn_usage();
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                let raw_response = json!({
                    "sessionId": turn.session_id,
                    "error": err.message_str(),
                });
                let mut fields = vec![
                    ("id", json!(expectation_id)),
                    ("attempt", json!(attempt)),
                    ("reason", json!(reason)),
                    ("error", json!(err.message_str())),
                    ("response", raw_response),
                ];
                append_turn_usage_fields(&mut fields, turn_usage.as_ref());
                let event = if turn_usage.is_some() {
                    "agent.response"
                } else {
                    append_missing_turn_usage_fields(&mut fields, turn.session_id);
                    "agent.turn_error"
                };
                writer.write_event("error", event, &fields)?;
            }
            return Err(err);
        }
    };
    let turn_usage = runner.take_last_turn_usage();
    let response_usage = turn_usage.as_ref().map(|turn_usage| turn_usage.usage);
    let missing_turn_usage = diagnostic_log.is_some() && turn_usage.is_none();
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        let raw_response = json!({
            "sessionId": turn.session_id,
            "text": response.clone(),
        });
        let mut fields: Vec<(&'static str, Value)> = vec![
            ("id", json!(expectation_id)),
            ("attempt", json!(attempt)),
            ("reason", json!(reason)),
            ("response", raw_response),
        ];
        append_turn_usage_fields(&mut fields, turn_usage.as_ref());
        if missing_turn_usage {
            // A response without usage violates the app-server turn contract,
            // so it is not logged as a completed `agent.response`.
            fields.push(("error", json!("missing evaluator turn usage")));
            append_missing_turn_usage_fields(&mut fields, turn.session_id);
            writer.write_event("error", "agent.turn_error", &fields)?;
        } else {
            writer.write_event("info", "agent.response", &fields)?;
        }
    }
    if missing_turn_usage {
        return Err(EvaluatorError::failure(
            EvaluatorFailureKind::UnknownAppServer,
            "missing evaluator turn usage",
        ));
    }
    let context_compacted = turn_usage
        .as_ref()
        .is_some_and(|turn_usage| !turn_usage.context_compaction_events.is_empty());
    Ok(RawTurnResponse {
        text: response,
        usage: response_usage,
        context_compacted,
    })
}

pub(crate) fn write_thread_lifecycle_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    lifecycle_log: &ThreadLifecycleLog,
    enforced_scope: &[String],
    model: Option<&str>,
    thinking: &str,
) -> Result<(), String> {
    write_thread_event(
        diagnostic_log,
        "info",
        lifecycle_log.event,
        &[
            ("threadId", json!(&lifecycle_log.session_id)),
            ("scope", json!(enforced_scope)),
            ("model", json!(model_label(model))),
            ("thinking", json!(thinking)),
            ("baseInstructions", json!(EVALUATOR_BASE_INSTRUCTIONS)),
            (
                "developerInstructions",
                json!(&lifecycle_log.developer_instructions),
            ),
        ],
    )
}

pub(crate) fn write_thread_restart_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    session_id: &str,
    expectation_id: Option<&str>,
    enforced_scope: &[String],
    model: Option<&str>,
    developer_instructions: &str,
    reason: &str,
) -> Result<(), String> {
    write_thread_event(
        diagnostic_log,
        "warn",
        "thread.restart",
        &[
            ("threadId", json!(session_id)),
            ("id", json!(expectation_id)),
            ("scope", json!(enforced_scope)),
            ("model", json!(model_label(model))),
            ("baseInstructions", json!(EVALUATOR_BASE_INSTRUCTIONS)),
            ("developerInstructions", json!(developer_instructions)),
            ("reason", json!(reason)),
        ],
    )
}

fn write_thread_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    level: &str,
    event: &str,
    fields: &[(&str, Value)],
) -> Result<(), String> {
    let Some(writer) = diagnostic_log.as_deref_mut() else {
        return Ok(());
    };
    writer
        .write_event(level, event, fields)
        .map_err(|err| err.to_string())
}

fn append_turn_usage_fields(
    fields: &mut Vec<(&'static str, Value)>,
    turn_usage: Option<&EvaluatorTurnUsage>,
) {
    let Some(EvaluatorTurnUsage {
        thread_id,
        turn_id,
        usage,
        token_usage_updates,
        context_compaction_events,
        ..
    }) = turn_usage
    else {
        return;
    };
    fields.push(("threadId", json!(thread_id)));
    fields.push(("turnId", json!(turn_id)));
    if !token_usage_updates.is_empty() {
        fields.push(("tokenUsageUpdates", json!(token_usage_updates)));
    } else {
        fields.push(("tokenUsage", token_usage_log_value(*usage)));
    }
    if !context_compaction_events.is_empty() {
        fields.push(("contextCompactionEvents", json!(context_compaction_events)));
    }
}

fn append_missing_turn_usage_fields(fields: &mut Vec<(&'static str, Value)>, session_id: &str) {
    fields.push(("threadId", json!(session_id)));
    fields.push(("turnId", json!("<missing>")));
    fields.push(("tokenUsage", token_usage_log_value(TokenUsage::default())));
    fields.push(("tokenUsageUnavailable", json!(true)));
}

fn token_usage_log_value(usage: TokenUsage) -> Value {
    json!({
        "totalTokens": usage.total_tokens,
        "inputTokens": usage.input_tokens,
        "cachedInputTokens": usage.cached_input_tokens,
        "outputTokens": usage.output_tokens,
        "reasoningOutputTokens": usage.reasoning_output_tokens,
    })
}

#[derive(Serialize)]
struct EvaluatorTurnLogRequest<'a> {
    #[serde(rename = "sessionId")]
    session_id: &'a str,
    prompt: &'a str,
    model: Option<&'a str>,
    thinking: &'a str,
}
