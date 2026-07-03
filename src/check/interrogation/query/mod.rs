use crate::check::core::{
    QueryResult, ResolvedExpectation, ERROR_INVALID_QUESTION, ERROR_SCOPE_TOO_NARROW,
    INTERNAL_ERROR_UNPARSABLE,
};
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::check::interrogation::{write_query_result_event, write_query_review_required_event};
use crate::check::{
    run_temporary_expectation_interrogation, CheckRunCaches,
    TemporaryExpectationInterrogationContext,
};
use crate::evaluator::{EvaluatorProgress, EvaluatorRunner};
use crate::logs::DiagnosticLogWriter;

#[derive(Clone, Copy)]
pub(crate) struct QueryRequest<'a> {
    pub(crate) question: &'a str,
    pub(crate) enforced_scope: &'a [String],
    pub(crate) expectation: QueryExpectationContext<'a>,
    pub(crate) progress: Option<&'a EvaluatorProgress>,
}

#[derive(Clone, Copy)]
pub(crate) struct QueryExpectationContext<'a> {
    pub(crate) expectation: &'a ResolvedExpectation,
}

pub(crate) fn run_query_with_runner<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    query: QueryRequest<'_>,
    runner: &mut R,
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    caches: &mut CheckRunCaches,
) -> Result<QueryResult, String> {
    // Query lifecycle start/finish events are emitted by
    // `check::command::execution::query` so they bracket scope parsing and
    // execution preparation as well as the evaluator turn managed here.
    let mut diagnostic_log = diagnostic_log;
    let mut verified_q_scope = query.enforced_scope.to_vec();
    let interrogation = run_temporary_expectation_interrogation(
        TemporaryExpectationInterrogationContext {
            runtime,
            runner,
            diagnostic_log: &mut diagnostic_log,
            caches,
            interrogation_run_state: state,
        },
        query.expectation.expectation,
        &mut verified_q_scope,
        query.progress,
    )?;
    finish_query_result(
        query.question,
        &mut diagnostic_log,
        QueryResult {
            answer: interrogation.answer,
        },
    )
}

fn finish_query_result(
    question: &str,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    result: QueryResult,
) -> Result<QueryResult, String> {
    assert_final_query_result_has_no_scope_too_narrow(&result)?;
    if let Some(reason) = query_human_review_reason(&result) {
        write_query_review_required_event(question, diagnostic_log, &result.answer, reason)
            .map_err(|err| err.to_string())?;
        return Ok(result);
    }
    write_query_result_event(question, diagnostic_log, &result.answer)
        .map_err(|err| err.to_string())?;
    Ok(result)
}

fn assert_final_query_result_has_no_scope_too_narrow(result: &QueryResult) -> Result<(), String> {
    if result.answer.error.as_deref() == Some(ERROR_SCOPE_TOO_NARROW) {
        return Err("internal error: forbidden final query scope error".to_string());
    }
    Ok(())
}

pub(crate) fn query_human_review_reason(result: &QueryResult) -> Option<&'static str> {
    match result.answer.error.as_deref() {
        Some(ERROR_SCOPE_TOO_NARROW) => {
            unreachable!("final query result cannot expose scope-too-narrow")
        }
        Some(ERROR_INVALID_QUESTION) => Some("invalid question"),
        Some(INTERNAL_ERROR_UNPARSABLE) => Some("unparsable evaluator response"),
        None => None,
        Some(_) => Some("unknown evaluator error"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::Cooldown;
    use crate::check::interrogation::state::{CheckRuntime, CheckTreeContext};
    use crate::config_types::{AgentConfig, CheckConfig, CheckHooksConfig, DEFAULT_DIFF_FROM};
    use crate::git::{staged_tree_oid, TreeSource};
    use crate::hash::full_scope;
    use crate::staged::StagedWorktreeView;
    use crate::token_usage_types::{EvaluatorTurnUsage, TokenUsage};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn ask_temporary_expectation_reports_answer_without_result_record() {
        let root = temp_root("ask-temporary-expectation");
        let config = CheckConfig {
            version: 1,
            agent: AgentConfig::implementation_default(),
            hooks: CheckHooksConfig::default(),
            expectations: Vec::new(),
        };
        let runtime = CheckRuntime::in_place(&root, &config, true);
        let expectation = ResolvedExpectation {
            number: 0,
            id: String::new(),
            display_id: "q".to_string(),
            question: "Does ask use a temporary xpec?".to_string(),
            expected_answer: String::new(),
            question_context: String::new(),
            diff_from: DEFAULT_DIFF_FROM.to_string(),
            target: None,
            question_answer_only: true,
            agent: config.agent.clone(),
            cooldown: Some(Cooldown {
                pass_seconds: None,
                fail_seconds: None,
            }),
        };
        let enforced_scope = full_scope();
        let request = QueryRequest {
            question: &expectation.question,
            enforced_scope: &enforced_scope,
            expectation: QueryExpectationContext {
                expectation: &expectation,
            },
            progress: None,
        };
        let mut runner =
            FakeQueryRunner::new(r#"{"q":{"answer":"yes","evidence":"checked visible files"}}"#);
        let mut state = InterrogationRunState::new(true).unwrap();
        let mut caches = CheckRunCaches::new();

        let result = run_query_with_runner(
            &runtime,
            request,
            &mut runner,
            None,
            &mut state,
            &mut caches,
        )
        .unwrap();

        let _ = fs::remove_dir_all(&root);
        assert_eq!(runner.ask_count, 1);
        assert_eq!(result.answer.observed, "yes");
        assert_eq!(result.answer.evidence, "checked visible files");
    }

    #[test]
    fn ask_temporary_expectation_does_not_write_git_backed_xpec_state() {
        let root = temp_git_root("ask-no-xpec-state");
        let config = CheckConfig {
            version: 1,
            agent: AgentConfig::implementation_default(),
            hooks: CheckHooksConfig::default(),
            expectations: Vec::new(),
        };
        let tree_source = TreeSource::Staged;
        let staged_view =
            StagedWorktreeView::apply_for_tree_source(&root, tree_source.clone()).unwrap();
        let checked_tree_oid = staged_tree_oid(&root).unwrap();
        let tree_context = CheckTreeContext {
            against_tree_oid: checked_tree_oid.clone(),
            checked_tree_oid,
            checked_file_count: 0,
        };
        let runtime = CheckRuntime::materialized(
            &root,
            &staged_view,
            &tree_source,
            tree_context,
            &config,
            true,
        );
        let expectation = ResolvedExpectation {
            number: 0,
            id: String::new(),
            display_id: "q".to_string(),
            question: "Does ask avoid xpec state?".to_string(),
            expected_answer: String::new(),
            question_context: String::new(),
            diff_from: DEFAULT_DIFF_FROM.to_string(),
            target: None,
            question_answer_only: true,
            agent: config.agent.clone(),
            cooldown: None,
        };
        let enforced_scope = full_scope();
        let request = QueryRequest {
            question: &expectation.question,
            enforced_scope: &enforced_scope,
            expectation: QueryExpectationContext {
                expectation: &expectation,
            },
            progress: None,
        };
        let mut runner = FakeQueryRunner::new(
            r#"{"q":{"answer":"yes","evidence":"checked staged files","qScopeSuggestion":["."]}}"#,
        );
        let mut state = InterrogationRunState::new(true).unwrap();
        let mut caches = CheckRunCaches::new();

        let result = run_query_with_runner(
            &runtime,
            request,
            &mut runner,
            None,
            &mut state,
            &mut caches,
        )
        .unwrap();

        let xpec_state_dir = root.join(".git").join("canon").join("xpecs");
        assert_eq!(result.answer.observed, "yes", "{}", result.answer.evidence);
        assert!(!xpec_state_dir.exists());
        let _ = fs::remove_dir_all(&root);
    }

    struct FakeQueryRunner {
        response: String,
        ask_count: usize,
    }

    impl FakeQueryRunner {
        fn new(response: &str) -> FakeQueryRunner {
            FakeQueryRunner {
                response: response.to_string(),
                ask_count: 0,
            }
        }
    }

    impl EvaluatorRunner for FakeQueryRunner {
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
        ) -> Result<String, crate::evaluator::EvaluatorError> {
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
        ) -> Result<String, crate::evaluator::EvaluatorError> {
            self.ask_count += 1;
            Ok(self.response.clone())
        }

        fn take_last_turn_usage(&mut self) -> Option<EvaluatorTurnUsage> {
            Some(EvaluatorTurnUsage {
                thread_id: "session".to_string(),
                turn_id: "turn".to_string(),
                usage: TokenUsage::default(),
                token_usage_updates: Vec::new(),
                context_compaction_events: Vec::new(),
            })
        }
    }

    fn temp_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "canon-query-test-{label}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn temp_git_root(label: &str) -> PathBuf {
        let root = temp_root(label);
        git(&root, &["init", "--quiet"]);
        root
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
