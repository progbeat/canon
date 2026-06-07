use super::*;
use crate::check::ERROR_UNPARSABLE;
use crate::config_types::AgentConfig;
use crate::evaluator::{EvaluatorError, EvaluatorResponseParseCache, EvaluatorRunner};
use crate::token_usage_types::EvaluatorTurnUsage;
use std::fs;
use std::path::Path;

#[test]
fn schema_valid_evidence_file_refs_do_not_trigger_repair() {
    let root = temp_root("schema-valid-evidence");
    let mut runner = RunnerWithResponses::new(vec![
        r#"{"answer":"yes","evidence":"`src/hidden.rs` supports it.","qScopeSuggestion":["src/hidden.rs"]}"#,
    ]);
    let mut parser_cache = EvaluatorResponseParseCache::new();
    let mut diagnostic_log = None;

    let parsed = ask_once(
        &mut runner,
        &turn_context(),
        "question",
        &AgentConfig::default(),
        &["src/visible.rs".to_string()],
        &root,
        &mut parser_cache,
        &mut diagnostic_log,
        Some("expectation"),
    )
    .unwrap();

    assert_eq!(parsed.answer.error, None);
    assert_eq!(parsed.answer.answer, "yes");
    assert_eq!(parsed.answer.evidence, "`src/hidden.rs` supports it.");
    assert!(runner.responses.is_empty());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn malformed_repair_response_stays_unparsable() {
    let root = temp_root("unparsable");
    let mut runner = RunnerWithResponses::new(vec!["status", "still status"]);
    let mut parser_cache = EvaluatorResponseParseCache::new();
    let mut diagnostic_log = None;

    let parsed = ask_once(
        &mut runner,
        &turn_context(),
        "question",
        &AgentConfig::default(),
        &[".".to_string()],
        &root,
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

    let _ = fs::remove_dir_all(root);
}

fn turn_context() -> EvaluatorTurnContext<'static> {
    EvaluatorTurnContext {
        session_id: "session",
        model: None,
        thinking: "medium",
    }
}

fn temp_root(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "canon-turn-evidence-test-{}-{}",
        label,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
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
