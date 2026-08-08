use super::*;
use crate::check::interrogation::state::{CheckRuntime, CheckTreeContext};
use crate::config_types::{AgentConfig, CheckConfig, DEFAULT_DIFF_FROM};
use crate::git::TreeSource;
use crate::hash::full_scope;
use crate::materialization::TreeMaterializer;
use crate::token_usage::{EvaluatorTurnUsage, TokenUsage};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test] // xpec: l
fn ask_temporary_expectation_reports_answer_without_result_record() {
    let root = temp_root("ask-temporary-expectation");
    let config = CheckConfig {
        version: 1,
        agent: AgentConfig::implementation_default(),
        expectations: Vec::new(),
    };
    let runtime = CheckRuntime::in_place(&root, &config, true);
    let expectation = ResolvedExpectation {
        kind: crate::check::core::ResolvedExpectationKind::TemporaryQuery,
        display_id: "q".to_string(),
        to: crate::config_types::ExpectationTo::Agent,
        rank: 0,
        question: "Does ask use a temporary xpec?".to_string(),
        expected_answer: String::new(),
        question_context: String::new(),
        diff_from: DEFAULT_DIFF_FROM.to_string(),
        target: None,
        agent: config.agent.clone(),
        cooldown: None,
        q_scope: Default::default(),
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
    let mut caches = CheckRunCaches::new();
    let mut interrogation_session =
        InterrogationSession::new(true, caches.temporary_directory_allocator.clone()).unwrap();

    let result = run_query_with_runner(
        &runtime,
        request,
        &mut runner,
        None,
        &mut interrogation_session,
        &mut caches,
    )
    .unwrap();

    let _ = fs::remove_dir_all(&root);
    assert_eq!(result.answer.observed, "yes");
    assert_eq!(
        result.answer.evidence.as_deref(),
        Some("checked visible files")
    );
}

#[test] // xpec: l
fn ask_returns_agent_output_and_provenance() {
    let root = temp_git_root("ask-no-xpec-state");
    fs::write(root.join("suggested.txt"), "suggested\n").unwrap();
    fs::write(root.join("outside.txt"), "outside\n").unwrap();
    git(&root, &["add", "suggested.txt", "outside.txt"]);
    let config = CheckConfig {
        version: 1,
        agent: AgentConfig::implementation_default(),
        expectations: Vec::new(),
    };
    let tree_source = TreeSource::Staged;
    let tree_materializer =
        TreeMaterializer::apply_for_tree_source(&root, tree_source.clone()).unwrap();
    let checked_tree_oid = tree_source.tree_oid_for_prompt_diff(&root).unwrap();
    let tree_context = CheckTreeContext {
        against_tree_oid: checked_tree_oid.clone(),
        checked_tree_oid,
        head_tree_oid: None,
        explicit_diff_from_tree_oids: Default::default(),
        checked_file_count: 2,
        prompt_git_environment: Vec::new(),
    };
    let runtime = CheckRuntime::materialized(
        &root,
        &tree_materializer,
        &tree_source,
        tree_context,
        &config,
        true,
    );
    let expectation = ResolvedExpectation {
        kind: crate::check::core::ResolvedExpectationKind::TemporaryQuery,
        display_id: "q".to_string(),
        to: crate::config_types::ExpectationTo::Agent,
        rank: 0,
        question: "Does ask avoid xpec state?".to_string(),
        expected_answer: String::new(),
        question_context: String::new(),
        diff_from: DEFAULT_DIFF_FROM.to_string(),
        target: None,
        agent: config.agent.clone(),
        cooldown: None,
        q_scope: Default::default(),
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
        r#"{"q":{"answer":"yes","evidence":"checked staged files","qScopeSuggestion":["suggested.txt"]}}"#,
    );
    let mut caches = CheckRunCaches::new();
    let mut interrogation_session =
        InterrogationSession::new(true, caches.temporary_directory_allocator.clone()).unwrap();

    let result = run_query_with_runner(
        &runtime,
        request,
        &mut runner,
        None,
        &mut interrogation_session,
        &mut caches,
    )
    .unwrap();

    assert_eq!(
        result.answer.observed, "yes",
        "{:?}",
        result.answer.evidence
    );
    assert_eq!(
        result.answer.q_scope_suggestion,
        Some(vec!["suggested.txt".to_string()])
    );
    assert_eq!(result.diff_from.as_deref(), Some(DEFAULT_DIFF_FROM));
    assert!(result
        .diff_from_tree_oid_abbrev
        .as_deref()
        .is_some_and(|oid| !oid.is_empty()));
    let _ = fs::remove_dir_all(&root);
}

pub(super) struct FakeQueryRunner {
    turn_results: VecDeque<Result<String, crate::evaluator::EvaluatorError>>,
    started_thread_count: usize,
    pub(super) ask_thread_ids: Vec<String>,
}

impl FakeQueryRunner {
    fn new(response: &str) -> FakeQueryRunner {
        Self::with_turn_results(vec![Ok(response.to_string())])
    }

    pub(super) fn with_turn_results(
        turn_results: Vec<Result<String, crate::evaluator::EvaluatorError>>,
    ) -> FakeQueryRunner {
        FakeQueryRunner {
            turn_results: turn_results.into(),
            started_thread_count: 0,
            ask_thread_ids: Vec::new(),
        }
    }
}

impl EvaluatorRunner for FakeQueryRunner {
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
    ) -> Result<String, crate::evaluator::EvaluatorError> {
        self.started_thread_count += 1;
        Ok(format!("thread-{}", self.started_thread_count))
    }

    fn ask(
        &mut self,
        thread_id: &str,
        _task_input: &str,
        _model: Option<&str>,
        _thinking: &str,
        _output_schema: &serde_json::Value,
        _dynamic_tool_handler: Option<&mut dyn crate::evaluator::EvaluatorDynamicToolHandler>,
    ) -> Result<String, crate::evaluator::EvaluatorError> {
        self.ask_thread_ids.push(thread_id.to_string());
        self.turn_results
            .pop_front()
            .expect("fake query runner has a turn result")
    }

    fn take_last_turn_usage(&mut self) -> Option<EvaluatorTurnUsage> {
        Some(EvaluatorTurnUsage {
            thread_id: self
                .ask_thread_ids
                .last()
                .cloned()
                .unwrap_or_else(|| "thread".to_string()),
            turn_id: "turn".to_string(),
            usage: TokenUsage::default(),
            token_usage_updates: Vec::new(),
            context_compaction_events: Vec::new(),
        })
    }

    fn set_progress_reporter(&mut self, _progress: Option<crate::evaluator::EvaluatorProgress>) {}
}

pub(super) fn temp_root(label: &str) -> PathBuf {
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
    // xpec: l
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}
