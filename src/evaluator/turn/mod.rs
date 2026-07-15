use crate::check::{
    CheckRecord, CheckRecordOutcome, CheckResult, ParsedAnswer, ResolvedExpectation,
};
use crate::config_types::AgentConfig;
use crate::evaluator::EvaluatorError;

mod attempt;
mod logging;
mod parse;
mod types;

pub(crate) use attempt::ask_once;
pub(crate) use logging::{write_thread_lifecycle_event, write_thread_restart_event};
pub(crate) use types::{
    EvaluatorTurnContext, ParsedTurnResponse, ThreadLifecycleLog, ThreadReuseLogContext,
};

pub(crate) fn evaluator_models(agent: &AgentConfig) -> Vec<Option<String>> {
    if agent.models.is_empty() {
        return vec![None];
    }
    agent.models.iter().cloned().map(Some).collect()
}

pub(crate) fn effective_thinking<'a>(
    _agent: &'a AgentConfig,
    expectation: &'a ResolvedExpectation,
) -> &'a str {
    &expectation.agent.thinking
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
    ShortIdResponse,
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
        self.is_model_technical() || matches!(self, EvaluatorFailureKind::ShortIdResponse)
    }
}

pub(crate) fn record_from_response(
    expectation: &ResolvedExpectation,
    response: ParsedAnswer,
    visible_tree_oid: String,
    diff_from: Option<String>,
    diff_from_tree_oid: Option<String>,
    diff_from_tree_oid_abbrev: Option<String>,
) -> Result<CheckRecord, String> {
    if expectation.expected_answer.is_empty() {
        return Err("cannot derive a check result without an expected answer".to_string());
    }
    let result = if response.error.is_some() {
        CheckResult::Fail
    } else {
        CheckResult::from_expected_answer(&expectation.expected_answer, &response.observed)
    };
    let error = response.error.clone();
    let question_scope_suggestion = response.question_scope_suggestion.clone();
    CheckRecord::current_from_expectation(
        expectation,
        CheckRecordOutcome {
            result,
            observed: response.observed,
            error,
            evidence: response.evidence,
            scope: response.scope,
            question_scope_suggestion,
            visible_tree_oid,
            diff_from,
            diff_from_tree_oid,
            diff_from_tree_oid_abbrev,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::INTERNAL_ERROR_UNPARSABLE;
    use crate::evaluator::{EvaluatorResponseParseCache, EvaluatorRunner};
    use crate::token_usage_types::{EvaluatorTurnUsage, TokenUsage};
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};

    // xpec: mh
    #[test]
    fn schema_valid_evidence_file_refs_do_not_trigger_repair() {
        let root = temp_root("schema-valid-evidence");
        let mut runner = RunnerWithResponses::new(vec![
            r#"{"q":{"answer":"yes","evidence":"`src/hidden.rs` supports it.","qScopeSuggestion":["src/hidden.rs"]}}"#,
        ]);
        let mut parser_cache = EvaluatorResponseParseCache::new();
        let mut diagnostic_log = None;

        let parsed = ask_once(
            &mut runner,
            &turn_context(),
            "question",
            &AgentConfig::default(),
            crate::check::EvaluatorResponseSchemaScope::Restricted,
            &json!({"type": "object"}),
            "q",
            &[],
            &["src/visible.rs".to_string()],
            &root,
            &mut parser_cache,
            &mut diagnostic_log,
            Some("expectation"),
            None,
        )
        .unwrap();

        assert_eq!(parsed.answer.error, None);
        assert_eq!(parsed.answer.observed, "yes");
        assert_eq!(parsed.answer.evidence, "`src/hidden.rs` supports it.");
        assert!(runner.responses.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    // xpec: mh
    #[test]
    fn malformed_response_stays_unparsable_without_repair() {
        let root = temp_root("unparsable");
        let mut runner = RunnerWithResponses::new(vec!["status", "still status"]);
        let mut parser_cache = EvaluatorResponseParseCache::new();
        let mut diagnostic_log = None;

        let parsed = ask_once(
            &mut runner,
            &turn_context(),
            "question",
            &AgentConfig::default(),
            crate::check::EvaluatorResponseSchemaScope::WithoutQuestionScopeSuggestion,
            &json!({"type": "object"}),
            "q",
            &[],
            &[".".to_string()],
            &root,
            &mut parser_cache,
            &mut diagnostic_log,
            Some("expectation"),
            None,
        )
        .unwrap();

        assert_eq!(
            parsed.answer.error.as_deref(),
            Some(INTERNAL_ERROR_UNPARSABLE)
        );
        assert!(parsed
            .answer
            .evidence
            .contains("evaluator response could not be parsed"));
        assert_eq!(runner.responses.len(), 1);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_record_requires_expected_answer() {
        let expectation = ResolvedExpectation {
            number: 0,
            id: String::new(),
            display_id: "q".to_string(),
            to: crate::config_types::ExpectationTo::Agent,
            rank: 0,
            question: "Does ask report an answer?".to_string(),
            expected_answer: String::new(),
            question_context: String::new(),
            diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
            target: None,
            question_answer_only: true,
            agent: AgentConfig::default(),
            cooldown: None,
        };
        let err = record_from_response(
            &expectation,
            ParsedAnswer::answer("yes".to_string(), "`src/main.rs`".to_string(), None),
            "visible".to_string(),
            None,
            None,
            None,
        )
        .unwrap_err();

        assert_eq!(
            err,
            "cannot derive a check result without an expected answer"
        );
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
            _template_artifact_paths: &[PathBuf],
            _base_instructions: &str,
            _developer_instructions: &str,
            _agent: &AgentConfig,
            _model: Option<&str>,
            _thinking: &str,
            _scope: &[String],
            _dynamic_tools: &[serde_json::Value],
        ) -> Result<String, EvaluatorError> {
            Ok("session".to_string())
        }

        fn ask(
            &mut self,
            _session_id: &str,
            _prompt: &str,
            _model: Option<&str>,
            _thinking: &str,
            _output_schema: &serde_json::Value,
            _dynamic_tool_handler: Option<&mut dyn crate::evaluator::EvaluatorDynamicToolHandler>,
        ) -> Result<String, EvaluatorError> {
            Ok(self.responses.remove(0))
        }

        fn take_last_turn_usage(&mut self) -> Option<EvaluatorTurnUsage> {
            Some(EvaluatorTurnUsage {
                thread_id: "session".to_string(),
                turn_id: "turn".to_string(),
                usage: TokenUsage {
                    total_tokens: 1,
                    ..TokenUsage::default()
                },
                token_usage_updates: Vec::new(),
                context_compaction_events: Vec::new(),
            })
        }
    }
}
