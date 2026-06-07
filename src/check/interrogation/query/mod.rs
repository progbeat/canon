mod narrowing;
mod review;
mod turn;

use crate::check::core::QueryResult;
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::check::interrogation::{
    run_with_model_fallbacks, write_query_result_event, write_query_review_required_event,
};
use crate::evaluator::{EvaluatorError, EvaluatorRunner};
use crate::logs::DiagnosticLogWriter;

#[derive(Clone, Copy)]
pub(crate) struct QueryRequest<'a> {
    pub(crate) question: &'a str,
    pub(crate) enforced_scope: &'a [String],
}

pub(crate) fn run_query_with_runner<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    enforced_scope: &[String],
    runner: &mut R,
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
) -> Result<QueryResult, String> {
    // Query lifecycle start/finish events are emitted by
    // `check::command::execution::query` so they bracket scope parsing and
    // execution preparation as well as the evaluator turn managed here.
    let mut diagnostic_log = diagnostic_log;
    run_with_model_fallbacks(
        &runtime.config.agent,
        state,
        &mut diagnostic_log,
        None,
        |state, diagnostic_log, model| {
            ask_with_model(
                runtime,
                QueryRequest {
                    question,
                    enforced_scope,
                },
                runner,
                diagnostic_log,
                state,
                model,
            )
        },
    )
}

fn ask_with_model<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    query: QueryRequest<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    model: Option<&str>,
) -> Result<QueryResult, EvaluatorError> {
    // `canon check -q` uses the same evaluator input shape as normal checks.
    // q-scope suggestions are trusted only after an independent verification
    // turn returns a schema-valid answer under the suggested scope.
    let mut active_scope = query.enforced_scope.to_vec();
    let mut result = turn::ask_with_full_scope_retry(
        runtime,
        query.question,
        &mut active_scope,
        runner,
        diagnostic_log,
        state,
        model,
    )?;
    if let Some(proposed_scope) =
        narrowing::scope_for_verification(runtime, state, &active_scope, &result.answer)?
    {
        let mut verification_scope = proposed_scope.clone();
        let narrowed = turn::ask_with_full_scope_retry(
            runtime,
            query.question,
            &mut verification_scope,
            runner,
            diagnostic_log,
            state,
            model,
        )?;
        if narrowing::answer_is_accepted(&narrowed.answer, &proposed_scope) {
            result = narrowed;
            result.answer.question_scope_suggestion = None;
        }
    }
    if let Some(reason) = review::human_review_reason(&result) {
        write_query_review_required_event(query.question, diagnostic_log, &result.answer, reason)?;
        return Err(EvaluatorError::message(format!(
            "query requires human review: {}",
            reason
        )));
    }
    write_query_result_event(query.question, diagnostic_log, &result.answer)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::QueryResult;
    use crate::check::interrogation::state::CheckTreeContext;
    use crate::config_types::{AgentConfig, CheckConfig};
    use crate::git::{TreeSource, VisibleTreeOidCache};
    use crate::staged::StagedWorktreeView;
    use crate::token_usage_types::{EvaluatorTurnUsage, TokenUsage};
    use std::collections::{BTreeMap, VecDeque};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn query_ignores_invalid_question_scope_suggestion() {
        let (result, runner) = run_query_responses(vec![
            r#"{"answer":"original","evidence":"ok","qScopeSuggestion":["/absolute"]}"#,
        ]);

        assert_eq!(result.answer.answer, "original");
        assert_eq!(
            result.answer.question_scope_suggestion.as_deref(),
            Some(&["/absolute".to_string()][..])
        );
        assert_eq!(runner.scopes, vec![vec![".".to_string()]]);
    }

    #[test]
    fn query_keeps_original_suggestion_when_narrowing_is_not_accepted() {
        let (result, runner) = run_query_responses(vec![
            r#"{"answer":"original","evidence":"ok","qScopeSuggestion":["src/a.rs"]}"#,
            r#"{"error":"insufficient-evidence","evidence":"need more","qScopeSuggestion":["src/a.rs"]}"#,
            r#"{"answer":"full-scope","evidence":"ok","qScopeSuggestion":["src/a.rs"]}"#,
        ]);

        assert_eq!(result.answer.answer, "original");
        assert_eq!(
            result.answer.question_scope_suggestion.as_deref(),
            Some(&["src/a.rs".to_string()][..])
        );
        assert!(runner.responses.is_empty());
    }

    fn run_query_responses(responses: Vec<&str>) -> (QueryResult, RunnerWithResponses) {
        let root = temp_repo("canon-query-narrowing");
        init_git_repo(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.rs"), "pub fn a() {}\n").unwrap();
        fs::write(root.join("src/b.rs"), "pub fn b() {}\n").unwrap();
        fs::write(root.join("README.md"), "readme\n").unwrap();
        git(&root, &["add", "."]);

        let config = CheckConfig {
            version: 1,
            presets: BTreeMap::new(),
            agent: AgentConfig::implementation_default(),
            expectations: Vec::new(),
        };
        let tree_source = TreeSource::Staged;
        let mut runner = RunnerWithResponses::new(responses);
        let result = {
            let staged_view =
                StagedWorktreeView::apply_for_tree_source(&root, tree_source.clone()).unwrap();
            let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
            let checked_tree_oid = tree_source.tree_oid_for_prompt_diff(&root).unwrap();
            let tree_context = CheckTreeContext {
                checked_tree_oid: checked_tree_oid.clone(),
                against_tree_oid: checked_tree_oid,
                against_tree: tree_source.clone(),
                checked_file_count: visible_tree_oid_cache
                    .checked_file_count(&root, &tree_source)
                    .unwrap(),
            };
            let runtime = CheckRuntime::materialized(
                &root,
                &staged_view,
                &tree_source,
                tree_context,
                &config,
                true,
            );
            let mut state = InterrogationRunState::new(runtime.no_sandbox()).unwrap();
            run_query_with_runner(
                &runtime,
                "question",
                &[".".to_string()],
                &mut runner,
                None,
                &mut state,
            )
            .unwrap()
        };

        (result, runner)
    }

    fn temp_repo(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("{prefix}-{}-{unique}", std::process::id()))
    }

    fn init_git_repo(root: &Path) {
        fs::create_dir_all(root).unwrap();
        git(root, &["init", "--quiet"]);
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    struct RunnerWithResponses {
        responses: VecDeque<String>,
        scopes: Vec<Vec<String>>,
    }

    impl RunnerWithResponses {
        fn new(responses: Vec<&str>) -> RunnerWithResponses {
            RunnerWithResponses {
                responses: responses.into_iter().map(str::to_string).collect(),
                scopes: Vec::new(),
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
            scope: &[String],
        ) -> Result<String, EvaluatorError> {
            self.scopes.push(scope.to_vec());
            Ok(format!("session-{}", self.scopes.len()))
        }

        fn ask(
            &mut self,
            _session_id: &str,
            _prompt: &str,
            _model: Option<&str>,
            _thinking: &str,
        ) -> Result<String, EvaluatorError> {
            self.responses
                .pop_front()
                .ok_or_else(|| EvaluatorError::message("missing test response"))
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
