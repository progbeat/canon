use crate::logs::error::{DiagnosticLogError, DiagnosticLogResult};
use crate::logs::DiagnosticLogWriter;
use crate::token_usage::{EvaluatorTurnUsage, TokenUsage};
use serde::Serialize;
use serde_json::{json, Value};

pub(crate) fn write_agent_request_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    request: AgentTurnLogRequest<'_>,
) -> DiagnosticLogResult<()> {
    // [w,g2,Yq] The common JSONL renderer adds primary `processId` and
    // `invocationId` correlation fields. These raw expectation and evaluator
    // thread IDs then identify the exchange within that command run.
    let raw_request = serde_json::to_value(request).map_err(|source| DiagnosticLogError::Json {
        description: "evaluator turn request log",
        source,
    })?;
    let mut fields = agent_event_fields(expectation_id, attempt, reason);
    fields.push(("request", raw_request));
    diagnostic_log.emit_event("info", "agent.request", &fields)
}

pub(crate) fn write_agent_failure_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    thread_id: &str,
    error: &str,
    turn_usage: Option<&EvaluatorTurnUsage>,
) -> DiagnosticLogResult<()> {
    let raw_response = json!({
        "threadId": thread_id,
        "error": error,
    });
    let mut fields = agent_turn_fields(expectation_id, attempt, reason, raw_response);
    let Some(turn_usage) = turn_usage else {
        append_missing_turn_usage_fields(&mut fields, thread_id);
        return diagnostic_log.emit_event("error", "agent.turn_error", &fields);
    };
    // [gN] Usage availability does not turn a failed turn into a completed
    // response. Record the failure first, then its available usage separately.
    diagnostic_log.emit_event("error", "agent.turn_error", &fields)?;
    write_agent_token_usage_event(diagnostic_log, expectation_id, attempt, reason, turn_usage)
}

pub(crate) fn write_agent_response_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    thread_id: &str,
    response: &str,
    turn_usage: &EvaluatorTurnUsage,
) -> DiagnosticLogResult<()> {
    let fields = agent_turn_fields(
        expectation_id,
        attempt,
        reason,
        agent_text_response(thread_id, response),
    );
    diagnostic_log.emit_event("info", "agent.response", &fields)?;
    write_agent_token_usage_event(diagnostic_log, expectation_id, attempt, reason, turn_usage)
}

pub(crate) fn write_agent_missing_usage_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    thread_id: &str,
    response: &str,
) -> DiagnosticLogResult<()> {
    let mut fields = agent_turn_fields(
        expectation_id,
        attempt,
        reason,
        agent_text_response(thread_id, response),
    );
    fields.push(("error", json!("missing evaluator turn usage")));
    append_missing_turn_usage_fields(&mut fields, thread_id);
    diagnostic_log.emit_event("error", "agent.turn_error", &fields)
}

fn agent_text_response(thread_id: &str, response: &str) -> Value {
    json!({
        "threadId": thread_id,
        "text": response,
    })
}

fn agent_turn_fields(
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    response: Value,
) -> Vec<(&'static str, Value)> {
    let mut fields = agent_event_fields(expectation_id, attempt, reason);
    fields.push(("response", response));
    fields
}

fn agent_event_fields(
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
) -> Vec<(&'static str, Value)> {
    vec![
        ("id", json!(expectation_id)),
        ("attempt", json!(attempt)),
        ("reason", json!(reason)),
    ]
}

#[derive(Serialize)]
pub(crate) struct AgentTurnLogRequest<'a> {
    #[serde(rename = "threadId")]
    pub(crate) thread_id: &'a str,
    #[serde(rename = "taskInput")]
    pub(crate) task_input: &'a str,
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
}

fn write_agent_token_usage_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
    turn_usage: &EvaluatorTurnUsage,
) -> DiagnosticLogResult<()> {
    let EvaluatorTurnUsage {
        thread_id,
        turn_id,
        usage,
        context_compaction_events,
        ..
    } = turn_usage;
    let mut fields = agent_event_fields(expectation_id, attempt, reason);
    fields.extend([
        ("threadId", json!(thread_id)),
        ("turnId", json!(turn_id)),
        // [w] Persist the normalized turn counters explicitly. Raw app-server
        // updates remain transport data and are not duplicated in runtime logs.
        ("tokenUsage", token_usage_log_value(*usage)),
    ]);
    if !context_compaction_events.is_empty() {
        fields.push(("contextCompactionEvents", json!(context_compaction_events)));
    }
    diagnostic_log.emit_event("info", "agent.token_usage", &fields)
}

fn append_missing_turn_usage_fields(fields: &mut Vec<(&'static str, Value)>, thread_id: &str) {
    fields.push(("threadId", json!(thread_id)));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::DiagnosticLogPlan;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: gN,Yq
    fn failed_turn_with_usage_keeps_error_and_usage_as_separate_events() {
        let root = git_temp_root("diagnostic-logs-failed-turn-usage");
        let state_root = crate::state_paths::CanonStateRoot::resolve(&root).unwrap();
        let mut writer = writer_with_limit(&root, 8192);
        let usage = EvaluatorTurnUsage {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            usage: TokenUsage::default(),
            token_usage_updates: Vec::new(),
            context_compaction_events: Vec::new(),
        };

        write_agent_failure_event(
            &mut writer,
            Some("id"),
            1,
            "initial",
            "thread",
            "failed",
            Some(&usage),
        )
        .unwrap();
        let events = read_json_lines(&state_root.join("logs/0.jsonl"));

        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["event"], "agent.turn_error");
        assert_eq!(events[0]["response"]["error"], "failed");
        assert_eq!(events[1]["event"], "agent.token_usage");
        fs::remove_dir_all(root).unwrap();
    }

    fn git_temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("canon-test-{name}-{}-{unique}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("init")
            .output()
            .unwrap();
        // xpec: gN,Yq
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        root
    }

    fn writer_with_limit(root: &Path, max_bytes: u64) -> DiagnosticLogWriter {
        let configured = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["config", "canon.logs.maxSize", &max_bytes.to_string()])
            .output()
            .unwrap();
        // xpec: gN,Yq
        assert!(configured.status.success());
        DiagnosticLogWriter::create_from_plan(root, DiagnosticLogPlan::prepare(root)).unwrap()
    }

    fn read_json_lines(path: &Path) -> Vec<Value> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}
