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
use crate::git::VisibleTreeOidCache;
use crate::hash::full_scope;
use crate::history::HistoryCache;
use crate::logs::DiagnosticLogWriter;
use crate::scope::sanitize_scope;

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
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
    break_after_tokens: Option<u64>,
) -> Result<InterrogationResult, String> {
    let mut interrogation = interrogate_or_error_record(
        call.call(),
        runner,
        diagnostic_log,
        interrogation_run_state,
        history_cache,
        visible_tree_oid_cache,
    )?;
    let should_stop_after_current_expectation =
        turn_exceeds_break_after_tokens(&interrogation, break_after_tokens)
            || turn_has_context_compaction(&interrogation);
    if should_retry_full_scope_after_error(
        interrogation.record.error.as_deref(),
        call.enforced_scope,
    ) || restricted_non_pass_needs_full_scope_confirmation(
        &interrogation.record,
        call.enforced_scope,
    ) {
        // Restricted insufficient-evidence is not final. A restricted non-pass
        // answer is also confirmed once at full scope so a stale q-scope cannot
        // be the sole basis for reporting a project violation.
        *call.enforced_scope = full_scope();
        interrogation = interrogate_or_error_record(
            call.call(),
            runner,
            diagnostic_log,
            interrogation_run_state,
            history_cache,
            visible_tree_oid_cache,
        )?;
        interrogation.stop_after_current_expectation |= should_stop_after_current_expectation;
    } else if should_stop_after_current_expectation {
        return Ok(interrogation);
    }
    Ok(interrogation)
}

pub(crate) fn interrogate_or_error_record<R: EvaluatorRunner>(
    call: InterrogationCall<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    interrogation_run_state: &mut InterrogationRunState,
    history_cache: &mut HistoryCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<InterrogationResult, String> {
    match interrogate_expectation_with_model_fallbacks(
        call.runtime,
        call.expectation,
        runner,
        diagnostic_log,
        interrogation_run_state,
        history_cache,
        call.scope,
    ) {
        Ok(interrogation) => Ok(interrogation),
        Err(err) => Ok(InterrogationResult {
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
        }),
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

pub(crate) fn restricted_non_pass_needs_full_scope_confirmation(
    record: &CheckRecord,
    scope: &[String],
) -> bool {
    scope != full_scope() && record.error.is_none() && !record.passed()
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

pub(crate) fn narrowed_scope_is_accepted(original: &CheckRecord, narrowed: &CheckRecord) -> bool {
    // Acceptance means the q-scope suggestion graduated from evaluator claim
    // to verified reusable q-scope. Interrogation Policy requires the
    // independent verification turn to produce a schema-valid answer. A
    // different answer means the suggested scope did not preserve the answer it
    // was meant to support for this tree.
    narrowed.error.is_none() && narrowed.observed == original.observed
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
        restricted_non_pass_needs_full_scope_confirmation,
        suggested_scope_is_at_least_25_percent_smaller,
    };
    use crate::check::core::{CheckRecord, CheckResult};
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
    fn restricted_non_pass_answer_needs_full_scope_confirmation() {
        let restricted_scope = vec!["src".to_string()];
        let non_pass = test_record("no", CheckResult::Fail, None);
        let pass = test_record("yes", CheckResult::Pass, None);
        let error = test_record(
            "insufficient-evidence",
            CheckResult::Fail,
            Some("insufficient-evidence"),
        );

        assert!(restricted_non_pass_needs_full_scope_confirmation(
            &non_pass,
            &restricted_scope
        ));
        assert!(!restricted_non_pass_needs_full_scope_confirmation(
            &non_pass,
            &full_scope()
        ));
        assert!(!restricted_non_pass_needs_full_scope_confirmation(
            &pass,
            &restricted_scope
        ));
        assert!(!restricted_non_pass_needs_full_scope_confirmation(
            &error,
            &restricted_scope
        ));
    }

    #[test]
    fn narrowed_scope_acceptance_requires_same_answer() {
        let original = test_record("yes", CheckResult::Pass, None);
        let matching = test_record("yes", CheckResult::Pass, None);
        let conflicting = test_record("no", CheckResult::Fail, None);
        let error = test_record(
            "insufficient-evidence",
            CheckResult::Fail,
            Some("insufficient-evidence"),
        );

        assert!(narrowed_scope_is_accepted(&original, &matching));
        assert!(!narrowed_scope_is_accepted(&original, &conflicting));
        assert!(!narrowed_scope_is_accepted(&original, &error));
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
            against_tree: source.clone(),
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
