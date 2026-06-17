use crate::check::core::errors::error_record_from_interrogation_error;
use crate::check::core::{CheckRecord, InterrogationResult, SelectedExpectation};
use crate::check::interrogation::state::{
    should_retry_full_scope_after_error, CheckRuntime, InterrogationRunState,
};
use crate::check::interrogation::{
    interrogate_expectation_with_model_fallbacks, scope_narrowing_log_fields,
};
use crate::config_types::AgentConfig;
use crate::evaluator::EvaluatorRunner;
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::hash::full_scope;
use crate::logs::DiagnosticLogWriter;
use crate::scope::sanitize_scope;
use crate::xpec_state::XpecStateCache;

pub(crate) struct InterrogationCall<'a> {
    pub(crate) runtime: &'a CheckRuntime<'a>,
    pub(crate) expectation: &'a SelectedExpectation,
    pub(crate) scope: &'a [String],
}

pub(crate) struct ScopedInterrogation<'a> {
    pub(crate) runtime: &'a CheckRuntime<'a>,
    pub(crate) expectation: &'a SelectedExpectation,
    pub(crate) enforced_scope: &'a mut Vec<String>,
}

pub(crate) struct PolicyInterrogationResult {
    interrogation: InterrogationResult,
    follow_up_used: bool,
}

impl PolicyInterrogationResult {
    pub(crate) fn interrogation(&self) -> &InterrogationResult {
        &self.interrogation
    }

    pub(crate) fn into_interrogation(self) -> InterrogationResult {
        self.interrogation
    }

    fn follow_up_used(&self) -> bool {
        self.follow_up_used
    }
}

impl<'a> ScopedInterrogation<'a> {
    fn call(&self) -> InterrogationCall<'_> {
        InterrogationCall {
            runtime: self.runtime,
            expectation: self.expectation,
            scope: self.enforced_scope,
        }
    }
}

pub(crate) fn interrogate_with_full_scope_retry<R: EvaluatorRunner>(
    call: ScopedInterrogation<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    interrogation_run_state: &mut InterrogationRunState,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    break_after_tokens: Option<u64>,
) -> Result<PolicyInterrogationResult, String> {
    let mut interrogation = interrogate_or_error_record(
        call.call(),
        runner,
        diagnostic_log,
        interrogation_run_state,
        xpec_state,
        visible_tree_oid_cache,
    )?;
    let should_stop_after_current_expectation =
        turn_exceeds_break_after_tokens(&interrogation, break_after_tokens)
            || turn_has_context_compaction(&interrogation);
    if should_retry_full_scope_after_error(
        interrogation.record.error.as_deref(),
        call.enforced_scope,
    ) {
        // Restricted ScopeTooNarrow is not final. The single policy follow-up
        // retries it once at full scope.
        *call.enforced_scope = full_scope();
        interrogation = interrogate_or_error_record(
            call.call(),
            runner,
            diagnostic_log,
            interrogation_run_state,
            xpec_state,
            visible_tree_oid_cache,
        )?;
        interrogation.stop_after_current_expectation |= should_stop_after_current_expectation;
        return Ok(PolicyInterrogationResult {
            interrogation,
            follow_up_used: true,
        });
    } else if should_stop_after_current_expectation {
        return Ok(PolicyInterrogationResult {
            interrogation,
            follow_up_used: false,
        });
    }
    Ok(PolicyInterrogationResult {
        interrogation,
        follow_up_used: false,
    })
}

pub(crate) fn interrogate_or_error_record<R: EvaluatorRunner>(
    call: InterrogationCall<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    interrogation_run_state: &mut InterrogationRunState,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<InterrogationResult, String> {
    match interrogate_expectation_with_model_fallbacks(
        call.runtime,
        call.expectation,
        runner,
        diagnostic_log,
        interrogation_run_state,
        xpec_state,
        call.scope,
    ) {
        Ok(interrogation) => Ok(interrogation),
        Err(err) => {
            let interrupted = err == "interrupted";
            Ok(InterrogationResult {
                record: error_record_from_interrogation_error(
                    call.runtime,
                    &call.expectation.agent,
                    call.expectation,
                    call.scope,
                    &err,
                    visible_tree_oid_cache,
                )?,
                turn_usage: None,
                context_compacted: false,
                stop_after_current_expectation: false,
                interrupted,
            })
        }
    }
}

pub(crate) fn turn_exceeds_break_after_tokens(
    interrogation: &InterrogationResult,
    break_after_tokens: Option<u64>,
) -> bool {
    let (Some(limit), Some(usage)) = (break_after_tokens, interrogation.turn_usage) else {
        return false;
    };
    usage.input_tokens.saturating_add(usage.output_tokens) > limit
}

pub(crate) fn turn_has_context_compaction(interrogation: &InterrogationResult) -> bool {
    interrogation.context_compacted
}

pub(crate) fn initial_visible_scope_for_expectation(
    root: &std::path::Path,
    _tree_source: &TreeSource,
    expectation: &SelectedExpectation,
    xpec_state: &mut XpecStateCache,
    _visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Vec<String>, String> {
    // Fresh interrogation starts from the stored q-scope. If no q-scope is
    // stored, it starts from full project scope. The actual visible scope is
    // formed later by appending the expectation agent's configured ignore
    // patterns as excluding pathspec items.
    // This scopes the materialized tree and detailed diff, not every prompt
    // signal: prompt rendering also includes an unscoped diff summary, so a
    // changed path outside this scope is still visible enough for the evaluator
    // to report ScopeTooNarrow when it needs the hidden details.
    // `target` is intentionally not an input; target-specific behavior belongs
    // to evaluator prompt rendering.
    let stored_q_scope = xpec_state.read_stored_q_scope(root, expectation)?;
    Ok(initial_scope_from_stored_q_scope(stored_q_scope))
}

pub(crate) fn initial_scope_from_stored_q_scope(
    stored_q_scope: Option<Vec<String>>,
) -> Vec<String> {
    stored_q_scope.unwrap_or_else(full_scope)
}

pub(crate) fn question_scope_suggestion_scope_for_unused_follow_up(
    runtime: &CheckRuntime<'_>,
    agent: &AgentConfig,
    result: &PolicyInterrogationResult,
    current_scope: &[String],
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Option<Vec<String>>, String> {
    // `ScopeTooNarrow` full-scope retry and q-scope verification share the
    // Interrogation Policy's single follow-up budget. If the retry already
    // consumed that budget, no q-scope verification turn is allowed.
    if result.follow_up_used() || result.interrogation.record.error.is_some() {
        return Ok(None);
    }
    question_scope_suggestion_scope_for_independent_verification(
        runtime,
        agent,
        result
            .interrogation
            .record
            .question_scope_suggestion
            .as_deref(),
        current_scope,
        visible_tree_oid_cache,
    )
}

pub(crate) fn question_scope_suggestion_scope_for_independent_verification(
    runtime: &CheckRuntime<'_>,
    agent: &AgentConfig,
    suggestion: Option<&[String]>,
    current_scope: &[String],
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Option<Vec<String>>, String> {
    // Glossary-level q-scope suggestions are evaluator-provided claims. This
    // helper implements only the Interrogation Policy gate for whether such a
    // claim is worth an independent verification turn: at least 25% fewer
    // visible files. The response JSON Schema does not require repo-relative
    // or semantically sufficient paths; sufficiency is established only when
    // the independent verification produces an answer. Returning `None` leaves
    // the evaluator's claim unverified; it does not redefine what a q-scope
    // suggestion is.
    let Some(suggestion) = suggestion else {
        return Ok(None);
    };
    let suggested_scope = match sanitize_scope(suggestion) {
        Ok(scope) => scope,
        Err(_) => return Ok(None),
    };
    if scope_targets_hidden_control_path(&suggested_scope) {
        return Ok(None);
    }
    if runtime
        .visible_tree_oid(visible_tree_oid_cache, agent, &suggested_scope)
        .is_err()
    {
        return Ok(None);
    }
    let current_count = runtime.visible_file_count(visible_tree_oid_cache, agent, current_scope)?;
    let suggested_count =
        match runtime.visible_file_count(visible_tree_oid_cache, agent, &suggested_scope) {
            Ok(count) => count,
            Err(_) => return Ok(None),
        };
    if suggested_scope_is_at_least_25_percent_smaller(current_count, suggested_count) {
        Ok(Some(suggested_scope))
    } else {
        Ok(None)
    }
}

fn suggested_scope_is_at_least_25_percent_smaller(
    current_count: usize,
    suggested_count: usize,
) -> bool {
    suggested_count < current_count
        && suggested_count.saturating_mul(4) <= current_count.saturating_mul(3)
}

fn scope_targets_hidden_control_path(scope: &[String]) -> bool {
    scope.iter().any(|path| {
        path == ".canon"
            || path.starts_with(".canon/")
            || path == ".git"
            || path.starts_with(".git/")
    })
}

pub(crate) fn narrowed_scope_is_accepted(narrowed: &CheckRecord) -> bool {
    // Acceptance means the q-scope suggestion graduated from evaluator claim
    // to verified reusable q-scope. Interrogation Policy requires the
    // independent verification turn to produce a schema-valid answer.
    narrowed.error.is_none()
}

pub(crate) fn write_scope_narrowing_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    id: &str,
    enforced_scope: &[String],
    record_scope: &[String],
    accepted: bool,
) -> Result<(), String> {
    let Some(writer) = diagnostic_log.as_deref_mut() else {
        return Ok(());
    };
    writer
        .write_event(
            "info",
            "scope.narrowing",
            &scope_narrowing_log_fields(id, enforced_scope, record_scope, accepted),
        )
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        narrowed_scope_is_accepted, question_scope_suggestion_scope_for_independent_verification,
        suggested_scope_is_at_least_25_percent_smaller,
    };
    use crate::check::core::{CheckRecord, CheckResult, ERROR_SCOPE_TOO_NARROW};
    use crate::check::interrogation::state::{CheckRuntime, CheckTreeContext};
    use crate::config_types::{AgentConfig, CheckConfig};
    use crate::git::{TreeSource, VisibleTreeOidCache};
    use crate::hash::full_scope;
    use crate::staged::StagedWorktreeView;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn equal_zero_file_scope_is_not_smaller() {
        assert!(!suggested_scope_is_at_least_25_percent_smaller(0, 0));
    }

    #[test]
    fn initial_scope_uses_stored_q_scope_or_full_scope() {
        let stored = vec!["src/main.rs".to_string()];

        assert_eq!(
            super::initial_scope_from_stored_q_scope(Some(stored.clone())),
            stored
        );
        assert_eq!(super::initial_scope_from_stored_q_scope(None), full_scope());
    }

    #[test]
    fn narrowed_scope_acceptance_requires_answer() {
        let pass = test_record("yes", CheckResult::Pass, None);
        let fail = test_record("no", CheckResult::Fail, None);
        let error = test_record(
            ERROR_SCOPE_TOO_NARROW,
            CheckResult::Fail,
            Some(ERROR_SCOPE_TOO_NARROW),
        );
        assert!(narrowed_scope_is_accepted(&pass));
        assert!(narrowed_scope_is_accepted(&fail));
        assert!(!narrowed_scope_is_accepted(&error));
    }

    #[test]
    fn absent_suggestion_path_is_not_verified_for_narrowing() {
        let root = git_project("absent-q-scope-suggestion");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/present.rs"), "present\n").unwrap();
        fs::write(root.join("src/other.rs"), "other\n").unwrap();
        git(&root, &["add", "src/present.rs", "src/other.rs"]);
        let source = TreeSource::Staged;
        let agent = AgentConfig::default();
        let config = CheckConfig {
            version: 1,
            presets: Default::default(),
            agent: agent.clone(),
            expectations: Vec::new(),
        };
        let staged_view = StagedWorktreeView::apply_for_tree_source(&root, source.clone()).unwrap();
        let mut cache = VisibleTreeOidCache::new();
        let tree_context = CheckTreeContext {
            checked_tree_oid: source.tree_oid_for_prompt_diff(&root).unwrap(),
            against_tree_oid: source.tree_oid_for_prompt_diff(&root).unwrap(),
            checked_file_count: cache.checked_file_count(&root, &source).unwrap(),
        };
        let runtime =
            CheckRuntime::materialized(&root, &staged_view, &source, tree_context, &config, false);
        let current_scope = vec![".".to_string()];
        let suggestion = vec!["src/present.rs".to_string(), "src/missing.rs".to_string()];

        let proposed = question_scope_suggestion_scope_for_independent_verification(
            &runtime,
            &agent,
            Some(&suggestion),
            &current_scope,
            &mut cache,
        )
        .unwrap();

        assert!(proposed.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hidden_control_path_suggestion_is_not_verified_for_narrowing() {
        let root = git_project("hidden-control-q-scope-suggestion");
        fs::create_dir_all(root.join(".canon")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join(".canon/check.yml"), "version: 1\n").unwrap();
        fs::write(root.join("src/present.rs"), "present\n").unwrap();
        git(&root, &["add", ".canon/check.yml", "src/present.rs"]);
        let source = TreeSource::Staged;
        let agent = AgentConfig::default();
        let config = CheckConfig {
            version: 1,
            presets: Default::default(),
            agent: agent.clone(),
            expectations: Vec::new(),
        };
        let staged_view = StagedWorktreeView::apply_for_tree_source(&root, source.clone()).unwrap();
        let mut cache = VisibleTreeOidCache::new();
        let tree_context = CheckTreeContext {
            checked_tree_oid: source.tree_oid_for_prompt_diff(&root).unwrap(),
            against_tree_oid: source.tree_oid_for_prompt_diff(&root).unwrap(),
            checked_file_count: cache.checked_file_count(&root, &source).unwrap(),
        };
        let runtime =
            CheckRuntime::materialized(&root, &staged_view, &source, tree_context, &config, false);
        let current_scope = vec![".".to_string()];
        let suggestion = vec![".canon/check.yml".to_string()];

        let proposed = question_scope_suggestion_scope_for_independent_verification(
            &runtime,
            &agent,
            Some(&suggestion),
            &current_scope,
            &mut cache,
        )
        .unwrap();

        assert!(proposed.is_none());
        let _ = fs::remove_dir_all(root);
    }

    fn git_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!(
                "canon-policy-{}-{}-{}",
                name,
                process::id(),
                unique
            ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "--quiet"]);
        root
    }

    fn git(root: &std::path::Path, args: &[&str]) {
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

    fn test_record(observed: &str, result: CheckResult, error: Option<&str>) -> CheckRecord {
        CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: 0,
            result,
            question: Some("question".to_string()),
            expected_answer: Some("yes".to_string()),
            observed: observed.to_string(),
            error: error.map(str::to_string),
            evidence: "evidence".to_string(),
            scope: full_scope(),
            question_scope_suggestion: None,
            visible_tree_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            id: "expectation".to_string(),
            display_id: "e".to_string(),
        }
    }
}
