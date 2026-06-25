use super::model_fallback::write_model_fallback_events;
use crate::check::core::{InterrogationResult, SelectedExpectation};
use crate::check::interrogation::finalize_interrogation_response;
use crate::check::interrogation::state::{
    evaluator_thread_reuse_key, CheckRuntime, InterrogationRunState,
};
use crate::check::{evaluator_response_output_schema_for_scope, EvaluatorResponseSchemaScope};
use crate::config_types::{AgentConfig, AGAINST_TREE_DIFF_FROM, DEFAULT_DIFF_FROM};
use crate::evaluator::{
    ask_once as ask_evaluator_once, developer_instructions, effective_thinking,
    evaluator_base_instructions, evaluator_turn_prompt, is_context_window_failure,
    q_scope_is_full_project, session_failure_invalidates_thread, write_thread_lifecycle_event,
    write_thread_restart_event, BaseInstructionsContext, DeveloperInstructionsContext,
    EvaluatorError, EvaluatorResponseParseCache, EvaluatorRunner, EvaluatorTurnContext,
    ParsedTurnResponse, ThreadLifecycleLog,
};
use crate::logs::DiagnosticLogWriter;
use crate::scope::sanitize_scope;
use crate::xpec_state::{LastResult, XpecStateCache};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub(crate) struct ThreadTurnRequest<'a> {
    pub(crate) agent: &'a AgentConfig,
    pub(crate) enforced_scope: &'a [String],
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
    pub(crate) expectation_id: Option<&'a str>,
    pub(crate) question_context: &'a str,
    pub(crate) diff_from_tree_oid: &'a str,
    pub(crate) prompt: &'a str,
    pub(crate) template_output_dir: &'a Path,
    pub(crate) template_artifact_paths: &'a [PathBuf],
    pub(crate) last_pass: Option<&'a LastResult>,
}

pub(crate) struct ResolvedDiffFrom<'a> {
    pub(crate) tree_oid: String,
    pub(crate) last_pass: Option<&'a LastResult>,
}

pub(crate) fn ask_with_reused_thread<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    request: ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let current_visible_tree_oid = runtime
        .visible_tree_oid(
            &mut state.visible_tree_oid_cache,
            request.agent,
            request.enforced_scope,
        )
        .map_err(EvaluatorError::message)?;
    let reuse_visible_tree_oid = request
        .last_pass
        .and_then(|last_pass| last_pass.visible_tree_oid.as_deref())
        .unwrap_or("");
    let session_key = evaluator_thread_reuse_key(
        request.agent,
        request.enforced_scope,
        request.model,
        reuse_visible_tree_oid,
        request.prompt,
        request.question_context,
        request.diff_from_tree_oid,
        runtime.checked_tree_oid(),
    )
    .map_err(EvaluatorError::message)?;
    // A restricted retry, q-scope verification, or different rendered diff
    // transcript misses this pool and starts a separate evaluator thread.
    let existing_session = state
        .thread_sessions_by_reuse_key
        .get(&session_key)
        .cloned();
    let had_existing_session = existing_session.is_some();
    let lifecycle_log = match existing_session {
        Some(existing) => thread_reuse_log(state, existing)?,
        None => start_thread_session(
            runtime,
            runner,
            state,
            &session_key,
            &current_visible_tree_oid,
            request,
        )?,
    };
    let mut session_id = lifecycle_log.session_id.clone();
    // Runtime logs expose the effective instructions for every thread start or
    // reuse, so thread behavior can be audited from the log without reading
    // derived state.
    write_thread_lifecycle_event(
        diagnostic_log,
        &lifecycle_log,
        request.enforced_scope,
        request.model,
        request.thinking,
    );
    let response =
        match ask_current_session(runtime, runner, &session_id, state, diagnostic_log, request) {
            Ok(response) => response,
            Err(err) if had_existing_session && is_context_window_failure(&err) => {
                clear_thread_sessions_after_failure(state);
                write_model_fallback_events(
                    diagnostic_log,
                    request.expectation_id,
                    request.model,
                    None,
                    err.message_str(),
                );
                write_thread_restart_event(
                    diagnostic_log,
                    &lifecycle_log,
                    request.expectation_id,
                    request.enforced_scope,
                    request.model,
                    err.message_str(),
                );
                let lifecycle_log = start_thread_session(
                    runtime,
                    runner,
                    state,
                    &session_key,
                    &current_visible_tree_oid,
                    request,
                )?;
                session_id = lifecycle_log.session_id.clone();
                write_thread_lifecycle_event(
                    diagnostic_log,
                    &lifecycle_log,
                    request.enforced_scope,
                    request.model,
                    request.thinking,
                );
                match ask_current_session(
                    runtime,
                    runner,
                    &session_id,
                    state,
                    diagnostic_log,
                    request,
                ) {
                    Ok(response) => response,
                    Err(err) => return fail_after_session_error(state, err),
                }
            }
            Err(err) => return fail_after_session_error(state, err),
        };
    if !retire_thread_sessions_after_turn(state, runner.take_retired_sessions(), &session_id) {
        state
            .thread_sessions_by_reuse_key
            .insert(session_key, session_id);
    }
    Ok(response)
}

fn ask_current_session<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    session_id: &str,
    state: &mut InterrogationRunState,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    request: ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let Some(session_root) = state.session_roots_by_id.get(session_id).cloned() else {
        return Err(EvaluatorError::message(format!(
            "missing evaluator session root for session {}",
            session_id
        )));
    };
    if let Err(err) = state.activate_session_root(session_id) {
        state.clear_thread_sessions();
        return Err(EvaluatorError::message(err));
    }
    ask_in_thread(
        runtime,
        runner,
        session_id,
        session_root.as_path(),
        &mut state.parse_cache,
        diagnostic_log,
        request,
    )
}

fn ask_in_thread<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    session_id: &str,
    session_root: &Path,
    parse_cache: &mut EvaluatorResponseParseCache,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    request: ThreadTurnRequest<'_>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let turn = EvaluatorTurnContext {
        session_id,
        model: request.model,
        thinking: request.thinking,
    };
    let visible_scope = runtime
        .visible_scope(request.agent, request.enforced_scope)
        .map_err(EvaluatorError::message)?;
    let schema_scope = if runtime.evaluator_interrogations_never_hide_files() {
        EvaluatorResponseSchemaScope::WithoutQuestionScopeSuggestion
    } else {
        EvaluatorResponseSchemaScope::for_scope_with_question_scope_suggestion(
            request.enforced_scope,
        )
    };
    let output_schema = evaluator_response_output_schema_for_scope(schema_scope);
    // The evaluator turn boundary owns agent.request/agent.response runtime
    // log events for the single evaluator request made by this turn.
    ask_evaluator_once(
        runner,
        &turn,
        request.prompt,
        request.agent,
        schema_scope,
        &output_schema,
        &visible_scope,
        session_root,
        parse_cache,
        diagnostic_log,
        request.expectation_id,
    )
}

fn start_thread_session<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    state: &mut InterrogationRunState,
    session_key: &str,
    visible_tree_oid: &str,
    request: ThreadTurnRequest<'_>,
) -> Result<ThreadLifecycleLog, EvaluatorError> {
    let visible_scope = runtime
        .visible_scope(request.agent, request.enforced_scope)
        .map_err(EvaluatorError::message)?;
    let visible_file_count = runtime.visible_file_count(
        &mut state.visible_tree_oid_cache,
        request.agent,
        request.enforced_scope,
    )?;
    let session_root = runtime
        .session_root_for_scope(request.agent, request.enforced_scope, visible_tree_oid)
        .map_err(EvaluatorError::message)?;
    // Render the developer-instructions resource template. `question_context`
    // is the value for that template's `expectation.instructions` input slot,
    // not a second prompt or instruction template.
    let mut template_artifact_paths = request.template_artifact_paths.to_vec();
    let developer_instructions = developer_instructions(DeveloperInstructionsContext {
        root: runtime.root,
        template_output_dir: request.template_output_dir,
        template_artifact_paths: &mut template_artifact_paths,
        in_place: runtime.is_in_place(),
        diff_from_tree_oid: request.diff_from_tree_oid,
        checked_tree_oid: runtime.checked_tree_oid(),
        question_context: request.question_context,
        visible_scope: &visible_scope,
        checked_file_count: runtime.checked_file_count(),
        visible_file_count,
        last_pass: request.last_pass,
    })
    .map_err(EvaluatorError::message)?;
    let base_instructions = evaluator_base_instructions(BaseInstructionsContext {
        in_place: runtime.is_in_place(),
        full_scope: q_scope_is_full_project(request.enforced_scope),
    })
    .map_err(EvaluatorError::message)?;
    let session_isolation = if runtime.is_in_place() {
        // In-place mode starts the evaluator in the checked directory itself.
        // Moving that directory to an isolation path would make the evaluator
        // cwd a sandbox copy instead of the checked directory required by the
        // in-place canon.
        None
    } else {
        state
            .isolate_session_root(&session_root)
            .map_err(EvaluatorError::message)?
    };
    let session_cwd = session_isolation
        .as_ref()
        .map(|isolation| isolation.path())
        .unwrap_or(session_root.as_path());
    let created = match runner.start_session(
        session_cwd,
        &template_artifact_paths,
        &base_instructions,
        &developer_instructions,
        request.agent,
        request.model,
        request.thinking,
        request.enforced_scope,
    ) {
        Ok(created) => created,
        Err(err) => return fail_after_session_error(state, err),
    };
    state
        .session_base_instructions
        .insert(created.clone(), base_instructions.clone());
    state
        .session_instructions
        .insert(created.clone(), developer_instructions.clone());
    state
        .session_roots_by_id
        .insert(created.clone(), session_cwd.to_path_buf());
    if let Some(isolation) = session_isolation {
        state.session_isolations.insert(created.clone(), isolation);
    }
    state
        .thread_sessions_by_reuse_key
        .insert(session_key.to_string(), created.clone());
    Ok(ThreadLifecycleLog {
        event: "thread.start",
        session_id: created,
        base_instructions,
        developer_instructions,
    })
}

fn thread_reuse_log(
    state: &InterrogationRunState,
    session_id: String,
) -> Result<ThreadLifecycleLog, EvaluatorError> {
    let Some(base_instructions) = state.session_base_instructions.get(&session_id).cloned() else {
        return Err(EvaluatorError::message(format!(
            "missing base instructions for reused session {}",
            session_id
        )));
    };
    let Some(developer_instructions) = state.session_instructions.get(&session_id).cloned() else {
        return Err(EvaluatorError::message(format!(
            "missing developer instructions for reused session {}",
            session_id
        )));
    };
    Ok(ThreadLifecycleLog {
        event: "thread.reuse",
        session_id,
        base_instructions,
        developer_instructions,
    })
}

fn clear_thread_sessions_after_failure(state: &mut InterrogationRunState) {
    // Reuse applies only to successful, still-live evaluator threads for the
    // same model and visible-tree context. Technical app-server failures can
    // retire the backing process, so keeping the old session ID would point at
    // a stale or missing thread rather than preserving the same Codex thread.
    state.clear_thread_sessions();
}

fn retire_thread_sessions_after_turn(
    state: &mut InterrogationRunState,
    retired_sessions: Vec<String>,
    active_session_id: &str,
) -> bool {
    if retired_sessions.is_empty() {
        return false;
    }
    let retired_sessions = retired_sessions.into_iter().collect::<BTreeSet<_>>();
    state
        .thread_sessions_by_reuse_key
        .retain(|_, session_id| !retired_sessions.contains(session_id));
    state
        .session_instructions
        .retain(|session_id, _| !retired_sessions.contains(session_id));
    state
        .session_base_instructions
        .retain(|session_id, _| !retired_sessions.contains(session_id));
    state
        .session_roots_by_id
        .retain(|session_id, _| !retired_sessions.contains(session_id));
    state
        .session_isolations
        .retain(|session_id, _| !retired_sessions.contains(session_id));
    retired_sessions.contains(active_session_id)
}

fn fail_after_session_error<T>(
    state: &mut InterrogationRunState,
    err: EvaluatorError,
) -> Result<T, EvaluatorError> {
    if session_failure_invalidates_thread(&err) {
        clear_thread_sessions_after_failure(state);
    }
    Err(err)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn interrogate_expectation_with_model<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    expectation: &SelectedExpectation,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    xpec_state: &mut XpecStateCache,
    enforced_scope: &[String],
    model: Option<&str>,
) -> Result<InterrogationResult, EvaluatorError> {
    // Expectation mode may start from a last-pass restricted scope, but
    // after sanitization this path shares query mode's first-turn construction:
    // developer instructions and the turn prompt are rendered from
    // `resources/prompts/` plus runtime data.
    let enforced_scope = sanitize_scope(enforced_scope)?;
    let last_pass = if runtime.is_in_place() {
        None
    } else {
        xpec_state
            .read_last_pass(runtime.root, expectation)
            .map_err(EvaluatorError::message)?
    };
    ask_expectation_turn(
        runtime,
        expectation,
        runner,
        diagnostic_log,
        state,
        &enforced_scope,
        model,
        last_pass.as_ref(),
    )
}

#[allow(clippy::too_many_arguments)]
fn ask_expectation_turn<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    expectation: &SelectedExpectation,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationRunState,
    enforced_scope: &[String],
    model: Option<&str>,
    last_pass: Option<&LastResult>,
) -> Result<InterrogationResult, EvaluatorError> {
    let template_output_dir = state
        .prompt_template_output_dir_cache
        .path_for_check_invocation()
        .map_err(EvaluatorError::message)?;
    let diff_from = resolve_diff_from(runtime, expectation, last_pass)?;
    let mut template_artifact_paths = Vec::new();
    let prompt = evaluator_turn_prompt(
        runtime.root,
        &template_output_dir,
        &mut template_artifact_paths,
        &expectation.question,
        &expectation.expected_answer,
        runtime.is_in_place(),
        &expectation.diff_from,
        expectation.target.as_ref().map(|target| target.as_str()),
        diff_from.last_pass,
    )
    .map_err(EvaluatorError::message)?;
    let thinking = effective_thinking(&expectation.agent, expectation);
    let response = ask_with_reused_thread(
        runtime,
        runner,
        diagnostic_log,
        state,
        ThreadTurnRequest {
            agent: &expectation.agent,
            enforced_scope,
            model,
            thinking,
            expectation_id: Some(&expectation.id),
            // This is question-scoped canon config data. The implementation-owned
            // evaluator instruction source is the template in `resources/prompts/`;
            // this text is only a value embedded by that source.
            question_context: &expectation.question_context,
            diff_from_tree_oid: &diff_from.tree_oid,
            prompt: &prompt,
            template_output_dir: &template_output_dir,
            template_artifact_paths: &template_artifact_paths,
            last_pass: diff_from.last_pass,
        },
    )?;
    finalize_interrogation_response(
        runtime,
        expectation,
        diagnostic_log,
        state,
        enforced_scope,
        response,
    )
}

pub(crate) fn resolve_diff_from<'a>(
    runtime: &CheckRuntime<'_>,
    expectation: &SelectedExpectation,
    last_pass: Option<&'a LastResult>,
) -> Result<ResolvedDiffFrom<'a>, EvaluatorError> {
    if runtime.is_in_place() {
        // In-place mode has no Git-backed diff base. Its selected expectations
        // are validated before interrogation, and prompt rendering clears any
        // diff-only turn inputs for this mode.
        return Ok(ResolvedDiffFrom {
            tree_oid: runtime.checked_tree_oid().to_string(),
            last_pass: None,
        });
    }
    // This is the canon `diff-from` resolver for prompt-rendered Git diffs.
    // `:checkpoint` uses the last pass checked tree only while that tree object
    // exists in the repository, otherwise it falls back to the run's against
    // tree. `:against-tree` uses the against tree directly. Other values use
    // the same TreeSource resolution as check command `<TREE>` options.
    let diff_from = expectation.diff_from.as_str();
    if diff_from == DEFAULT_DIFF_FROM {
        return checkpoint_diff_base(runtime.root, last_pass, runtime.against_tree_oid());
    }
    if diff_from == AGAINST_TREE_DIFF_FROM {
        return Ok(ResolvedDiffFrom {
            tree_oid: runtime.against_tree_oid().to_string(),
            last_pass,
        });
    }
    Ok(ResolvedDiffFrom {
        tree_oid: explicit_diff_base_tree_oid(runtime.root, diff_from)?,
        last_pass,
    })
}

fn checkpoint_diff_base<'a>(
    root: &Path,
    last_pass: Option<&'a LastResult>,
    against_tree_oid: &str,
) -> Result<ResolvedDiffFrom<'a>, EvaluatorError> {
    if let Some(checked_tree_oid) =
        last_pass.and_then(|last_pass| last_pass.checked_tree_oid.as_deref())
    {
        if crate::git::git_object_oid_has_known_shape(checked_tree_oid)
            && crate::git::tree_object_exists(root, checked_tree_oid)
                .map_err(EvaluatorError::message)?
        {
            return Ok(ResolvedDiffFrom {
                tree_oid: checked_tree_oid.to_string(),
                last_pass,
            });
        }
    }
    Ok(ResolvedDiffFrom {
        tree_oid: against_tree_oid.to_string(),
        last_pass: None,
    })
}

fn explicit_diff_base_tree_oid(root: &Path, tree: &str) -> Result<String, EvaluatorError> {
    crate::git::TreeSource::resolve(root, tree, "diff-from")
        .and_then(|source| source.tree_oid_for_prompt_diff(root))
        .map_err(EvaluatorError::message)
}

#[cfg(test)]
mod tests {
    use super::checkpoint_diff_base;
    use crate::xpec_state::{LastResult, LastResultStatus};
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn checkpoint_diff_base_uses_existing_checkpoint_tree() {
        let root = git_project("checkpoint-existing");
        let checked_tree_oid = crate::git::staged_tree_oid(&root).unwrap();
        let last_pass = last_pass_with_checked_tree_oid(&checked_tree_oid);

        let resolved = checkpoint_diff_base(&root, Some(&last_pass), "against-tree").unwrap();

        assert_eq!(resolved.tree_oid, checked_tree_oid);
        assert!(resolved.last_pass.is_some());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn checkpoint_diff_base_ignores_missing_checkpoint_tree() {
        let root = git_project("checkpoint-missing");
        let missing_tree_oid = "ffffffffffffffffffffffffffffffffffffffff";
        let last_pass = last_pass_with_checked_tree_oid(missing_tree_oid);

        let resolved = checkpoint_diff_base(&root, Some(&last_pass), "against-tree").unwrap();

        assert_eq!(resolved.tree_oid, "against-tree");
        assert!(resolved.last_pass.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn checkpoint_diff_base_ignores_non_oid_checkpoint_tree() {
        let root = git_project("checkpoint-revspec");
        let last_pass = last_pass_with_checked_tree_oid("HEAD^{tree}");

        let resolved = checkpoint_diff_base(&root, Some(&last_pass), "against-tree").unwrap();

        assert_eq!(resolved.tree_oid, "against-tree");
        assert!(resolved.last_pass.is_none());
        let _ = fs::remove_dir_all(root);
    }

    fn last_pass_with_checked_tree_oid(checked_tree_oid: &str) -> LastResult {
        LastResult {
            response_timestamp: "1970-01-01T00:00:00Z".to_string(),
            updated_timestamp: "1970-01-01T00:00:00Z".to_string(),
            status: LastResultStatus::Pass,
            response: json!({
                "answer": "yes",
                "evidence": "`src/main.rs`",
                "qScopeSuggestion": ["."]
            }),
            q_scope: vec![".".to_string()],
            visible_scope: vec![".".to_string()],
            checked_tree_oid: Some(checked_tree_oid.to_string()),
            visible_tree_oid: None,
        }
    }

    fn git_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("canon-{name}-{}-{unique}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        git(&root, &["init"]);
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        git(&root, &["add", "src/main.rs"]);
        root
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
