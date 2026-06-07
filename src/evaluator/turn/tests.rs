use super::*;
use crate::check::{ERROR_INSUFFICIENT_EVIDENCE, ERROR_UNPARSABLE};
use crate::config_types::AgentConfig;
use crate::token_usage_types::EvaluatorTurnUsage;
use std::fs;
use std::path::Path;

#[test]
fn out_of_scope_evidence_after_repair_becomes_insufficient_evidence() {
    let mut runner = RunnerWithResponses::new(vec![
        r#"{"answer":"yes","evidence":"`src/hidden.rs` supports it.","qScopeSuggestion":["src/hidden.rs"]}"#,
        r#"{"answer":"yes","evidence":"`src/hidden.rs` still supports it.","qScopeSuggestion":["src/hidden.rs"]}"#,
    ]);
    let mut parser_cache = EvaluatorResponseParseCache::new();
    let mut diagnostic_log = None;

    let parsed = ask_once(
        &mut runner,
        &turn_context(),
        "question",
        &AgentConfig::default(),
        &["src/visible.rs".to_string()],
        None,
        &mut parser_cache,
        &mut diagnostic_log,
        Some("expectation"),
    )
    .unwrap();

    assert_eq!(
        parsed.answer.error.as_deref(),
        Some(ERROR_INSUFFICIENT_EVIDENCE)
    );
    assert!(parsed.answer.evidence.contains("outside the visible scope"));
}

#[test]
fn malformed_repair_response_stays_unparsable() {
    let mut runner = RunnerWithResponses::new(vec!["status", "still status"]);
    let mut parser_cache = EvaluatorResponseParseCache::new();
    let mut diagnostic_log = None;

    let parsed = ask_once(
        &mut runner,
        &turn_context(),
        "question",
        &AgentConfig::default(),
        &[".".to_string()],
        None,
        &mut parser_cache,
        &mut diagnostic_log,
        Some("expectation"),
    )
    .unwrap();

    assert_eq!(parsed.answer.error.as_deref(), Some(ERROR_UNPARSABLE));
    assert!(parsed
        .answer
        .evidence
        .contains("evaluator response could not be parsed"));
}

#[test]
fn nonexistent_evidence_file_after_repair_becomes_insufficient_evidence() {
    let root =
        std::env::temp_dir().join(format!("canon-turn-evidence-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    let mut runner = RunnerWithResponses::new(vec![
        r#"{"answer":"yes","evidence":"`src/deleted.rs` supports it.","qScopeSuggestion":["src/deleted.rs"]}"#,
        r#"{"answer":"yes","evidence":"`src/deleted.rs` still supports it.","qScopeSuggestion":["src/deleted.rs"]}"#,
    ]);
    let mut parser_cache = EvaluatorResponseParseCache::new();
    let mut diagnostic_log = None;

    let parsed = ask_once(
        &mut runner,
        &turn_context(),
        "question",
        &AgentConfig::default(),
        &[".".to_string()],
        Some(&root),
        &mut parser_cache,
        &mut diagnostic_log,
        Some("expectation"),
    )
    .unwrap();

    assert_eq!(
        parsed.answer.error.as_deref(),
        Some(ERROR_INSUFFICIENT_EVIDENCE)
    );

    let _ = fs::remove_dir_all(root);
}

fn turn_context() -> EvaluatorTurnContext<'static> {
    EvaluatorTurnContext {
        session_id: "session",
        model: None,
        thinking: "medium",
    }
}

struct RunnerWithResponses {
    responses: Vec<String>,
}

impl RunnerWithResponses {
    fn new(responses: Vec<&str>) -> RunnerWithResponses {
        RunnerWithResponses {
            responses: responses.into_iter().map(str::to_string).collect(),
        }
    }
}

impl EvaluatorRunner for RunnerWithResponses {
    fn start_session(
        &mut self,
        _session_cwd: &Path,
        _developer_instructions: &str,
        _agent: &AgentConfig,
        _model: Option<&str>,
        _thinking: &str,
        _scope: &[String],
    ) -> Result<String, EvaluatorError> {
        Ok("session".to_string())
    }

    fn ask(
        &mut self,
        _session_id: &str,
        _prompt: &str,
        _model: Option<&str>,
        _thinking: &str,
    ) -> Result<String, EvaluatorError> {
        Ok(self.responses.remove(0))
    }

    fn take_last_turn_usage(&mut self) -> Option<EvaluatorTurnUsage> {
        None
    }
}
