//! Executes evaluator turns from already-rendered task input.

#[cfg(test)]
use crate::check::ParsedAnswer;
#[cfg(test)]
use crate::check::ResolvedExpectation;
use crate::config_types::AgentConfig;
#[cfg(test)]
use crate::evaluator::EvaluatorError;

mod attempt;
mod failure;
mod logging;
mod parse;
mod record;
mod types;

pub(crate) use attempt::{
    ask_once, EvaluatorAttempt, EvaluatorAttemptReason, EvaluatorAttemptRequest,
    EvaluatorAttemptSequence,
};
pub(crate) use failure::{is_interrupted, is_technical_failure, EvaluatorFailureKind};
pub(crate) use logging::{write_thread_lifecycle_event, write_thread_restart_event};
pub(crate) use record::record_from_response;
pub(crate) use types::{
    EvaluatorTurnContext, ParsedTurnResponse, ThreadEvaluationLogContext, ThreadLifecycleLog,
};

pub(crate) fn evaluator_models(agent: &AgentConfig) -> Vec<Option<String>> {
    if agent.models.is_empty() {
        return vec![None];
    }
    agent.models.iter().cloned().map(Some).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::{EvaluationAnswer, INTERNAL_ERROR_UNPARSABLE};
    use crate::evaluator::{EvaluatorRunner, InvocationResponseParseMemo};
    use crate::token_usage::{EvaluatorTurnUsage, TokenUsage};
    use serde_json::json;
    use std::path::Path;

    // xpec: qv
    #[test]
    fn schema_valid_evidence_file_refs_do_not_trigger_repair() {
        let mut runner = RunnerWithResponses::new(vec![
            r#"{"q":{"answer":"yes","evidence":"`src/hidden.rs:1` supports it.","qScopeSuggestion":["src/hidden.rs"]}}"#,
        ]);
        let mut attempt_sequence = EvaluatorAttemptSequence::default();
        let mut response_parse_memo = InvocationResponseParseMemo::new();
        let mut diagnostic_log = None;

        let parsed = ask_once(
            &mut runner,
            &mut response_parse_memo,
            &mut diagnostic_log,
            EvaluatorAttemptRequest {
                attempt: attempt_sequence.next(EvaluatorAttemptReason::Initial),
                turn: &turn_context(),
                task_input: "question",
                schema_scope: crate::check::EvaluatorResponseSchemaScope::AutoRestricted,
                output_schema: &json!({"type": "object"}),
                short_id: "q",
                answered_short_ids: &[],
                expectation_id: Some("expectation"),
            },
            None,
        )
        .unwrap();

        assert_eq!(parsed.answer.error, None);
        assert_eq!(parsed.answer.observed, "yes");
        assert_eq!(
            parsed.answer.evidence.as_deref(),
            Some("`src/hidden.rs:1` supports it.")
        );
        assert!(runner.responses.is_empty());
    }

    // xpec: qv
    #[test]
    fn malformed_response_stays_unparsable_without_repair() {
        let mut runner = RunnerWithResponses::new(vec!["status", "still status"]);
        let mut attempt_sequence = EvaluatorAttemptSequence::default();
        let mut response_parse_memo = InvocationResponseParseMemo::new();
        let mut diagnostic_log = None;

        let parsed = ask_once(
            &mut runner,
            &mut response_parse_memo,
            &mut diagnostic_log,
            EvaluatorAttemptRequest {
                attempt: attempt_sequence.next(EvaluatorAttemptReason::Initial),
                turn: &turn_context(),
                task_input: "question",
                schema_scope: crate::check::EvaluatorResponseSchemaScope::FixedQScope,
                output_schema: &json!({"type": "object"}),
                short_id: "q",
                answered_short_ids: &[],
                expectation_id: Some("expectation"),
            },
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
            .as_deref()
            .is_some_and(|evidence| evidence.contains("evaluator response could not be parsed")));
        assert_eq!(runner.responses.len(), 1);
    }

    #[test] // xpec: Eg,l
    fn temporary_query_cannot_become_a_check_record() {
        let expectation = ResolvedExpectation {
            kind: crate::check::ResolvedExpectationKind::TemporaryQuery,
            display_id: "q".to_string(),
            to: crate::config_types::ExpectationTo::Agent,
            rank: 0,
            question: "Does ask report an answer?".to_string(),
            expected_answer: String::new(),
            question_context: String::new(),
            diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
            target: None,
            agent: AgentConfig::default(),
            cooldown: None,
            q_scope: Default::default(),
        };
        let result = record_from_response(
            &expectation,
            ParsedAnswer::answer(
                EvaluationAnswer::new("yes".to_string()),
                "`src/main.rs`".to_string(),
                None,
            ),
            Some("visible".to_string()),
            None,
            None,
            None,
        );

        assert!(result.is_err());
    }

    fn turn_context() -> EvaluatorTurnContext<'static> {
        EvaluatorTurnContext {
            thread_id: "thread",
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
        fn start_thread(
            &mut self,
            _thread_cwd: &Path,
            _template_artifact_directory: &Path,
            _rendered_base_text: &str,
            _rendered_developer_text: &str,
            _agent: &AgentConfig,
            _model: Option<&str>,
            _thinking: &str,
            _dynamic_tools: &[serde_json::Value],
        ) -> Result<String, EvaluatorError> {
            Ok("thread".to_string())
        }

        fn ask(
            &mut self,
            _thread_id: &str,
            _task_input: &str,
            _model: Option<&str>,
            _thinking: &str,
            _output_schema: &serde_json::Value,
            _dynamic_tool_handler: Option<&mut dyn crate::evaluator::EvaluatorDynamicToolHandler>,
        ) -> Result<String, EvaluatorError> {
            Ok(self.responses.remove(0))
        }

        fn take_last_turn_usage(&mut self) -> Option<EvaluatorTurnUsage> {
            Some(EvaluatorTurnUsage {
                thread_id: "thread".to_string(),
                turn_id: "turn".to_string(),
                usage: TokenUsage {
                    total_tokens: 1,
                    ..TokenUsage::default()
                },
                token_usage_updates: Vec::new(),
                context_compaction_events: Vec::new(),
            })
        }

        fn set_progress_reporter(
            &mut self,
            _progress: Option<crate::evaluator::EvaluatorProgress>,
        ) {
        }
    }
}
