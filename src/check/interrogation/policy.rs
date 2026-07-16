use crate::check::core::errors::{
    error_record_from_interrogation_error, InterrogationDiffProvenance,
};
use crate::check::core::{
    CheckRecord, InterrogationAnswer, InterrogationResult, ParsedAnswer, ResolvedExpectation,
    INTERNAL_ERROR_UNPARSABLE,
};
use crate::check::interrogation::session::interrogate_expectation_answer_with_model_fallbacks;
use crate::check::interrogation::session::resolve_diff_from;
use crate::check::interrogation::state::{CheckRuntime, InterrogationRunState};
use crate::check::interrogation::{
    interrogate_expectation_with_model_fallbacks, scope_narrowing_log_fields,
    InterrogationRequestKind, ModelFallbackInterrogation,
};
use crate::config_types::AgentConfig;
use crate::evaluator::EvaluatorProgress;
use crate::evaluator::EvaluatorRunner;
use crate::git::VisibleTreeOidCache;
use crate::hash::full_scope;
use crate::logs::DiagnosticLogWriter;
use crate::scope::{sanitize_scope, scope_is_within};
use crate::xpec_state::XpecStateCache;

// Interrogation Policy implementation map:
// - full vs restricted response schema selection, `ScopeTooNarrow`,
//   `InvalidQuestion`, `answer`, `evidence`, and `qScopeSuggestion` schema
//   parsing: `src/check/core/evaluator_response.rs`
// - initial q-scope, no-hide follow-up suppression, invalid qScopeSuggestion
//   rejection, 25%-smaller gate, and narrowed-scope acceptance: this file
// - check-run `ScopeTooNarrow` retry and q-scope verification sequencing:
//   `src/check/run/execute/expectation.rs`
// - `canon ask` retry and q-scope verification sequencing:
//   `src/check/interrogation/query/mod.rs`
// - evaluator model retry order: `src/check/interrogation/session/model_fallback.rs`
// - per-turn thinking, enforced response schema, prompt, and thread inputs:
//   `src/check/interrogation/session/thread.rs`
// - configured model list and thinking expansion for retries:
//   `src/check/interrogation/state.rs`
// A q-scope verification `ScopeTooNarrow` rejects the proposed narrowed scope;
// the initial answer remains the final response.
// A whole-policy audit needs all of these code paths; a narrower q-scope can
// verify only the policy clauses owned by the included files.

pub(crate) struct InterrogationCall<'a> {
    pub(crate) runtime: &'a CheckRuntime<'a>,
    pub(crate) expectation: &'a ResolvedExpectation,
    pub(crate) scope: &'a [String],
    pub(crate) request_kind: InterrogationRequestKind,
    pub(crate) progress: Option<&'a EvaluatorProgress>,
}

pub(crate) struct PolicyInterrogationResult {
    interrogation: InterrogationResult,
    follow_up_used: bool,
}

impl PolicyInterrogationResult {
    pub(crate) fn new(
        interrogation: InterrogationResult,
        follow_up_used: bool,
    ) -> PolicyInterrogationResult {
        PolicyInterrogationResult {
            interrogation,
            follow_up_used,
        }
    }

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

pub(crate) fn interrogate_or_error_record<R: EvaluatorRunner>(
    call: InterrogationCall<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    interrogation_run_state: &mut InterrogationRunState,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<InterrogationResult, String> {
    match interrogate_expectation_with_model_fallbacks(
        ModelFallbackInterrogation {
            runtime: call.runtime,
            expectation: call.expectation,
            enforced_scope: call.scope,
            request_kind: call.request_kind,
            progress: call.progress,
        },
        runner,
        diagnostic_log,
        interrogation_run_state,
        xpec_state,
    ) {
        Ok(interrogation) => Ok(interrogation),
        Err(err) => {
            let interrupted = err == "interrupted";
            let diff_provenance = interrogation_error_diff_provenance(&call, xpec_state)?;
            Ok(InterrogationResult {
                record: error_record_from_interrogation_error(
                    call.runtime,
                    &call.expectation.agent,
                    call.expectation,
                    call.scope,
                    &err,
                    diff_provenance,
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

pub(crate) fn git_backed_interrogation_diff_provenance(
    runtime: &CheckRuntime<'_>,
    expectation: &ResolvedExpectation,
    xpec_state: &mut XpecStateCache,
) -> Result<Option<InterrogationDiffProvenance>, String> {
    if runtime.is_in_place() {
        return Ok(None);
    }
    let last_pass = if expectation.id.is_empty() {
        None
    } else {
        xpec_state.read_last_pass(runtime.root, expectation)?
    };
    let diff_from = resolve_diff_from(runtime, expectation, last_pass.as_ref())
        .map_err(|err| err.to_string())?;
    let diff_from_tree_oid_abbrev =
        crate::git::abbreviate_git_oid(runtime.root, &diff_from.tree_oid)?;
    Ok(Some(InterrogationDiffProvenance {
        diff_from: expectation.diff_from.clone(),
        diff_from_tree_oid: diff_from.tree_oid,
        diff_from_tree_oid_abbrev,
    }))
}

fn interrogation_error_diff_provenance(
    call: &InterrogationCall<'_>,
    xpec_state: &mut XpecStateCache,
) -> Result<Option<InterrogationDiffProvenance>, String> {
    git_backed_interrogation_diff_provenance(call.runtime, call.expectation, xpec_state)
}

pub(crate) fn interrogate_or_error_answer<R: EvaluatorRunner>(
    call: InterrogationCall<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    interrogation_run_state: &mut InterrogationRunState,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<InterrogationAnswer, String> {
    match interrogate_expectation_answer_with_model_fallbacks(
        ModelFallbackInterrogation {
            runtime: call.runtime,
            expectation: call.expectation,
            enforced_scope: call.scope,
            request_kind: call.request_kind,
            progress: call.progress,
        },
        runner,
        diagnostic_log,
        interrogation_run_state,
        xpec_state,
    ) {
        Ok(answer) => Ok(answer),
        Err(err) => {
            let interrupted = err == "interrupted";
            let visible_tree_oid = call.runtime.visible_tree_oid(
                visible_tree_oid_cache,
                &call.expectation.agent,
                call.scope,
            )?;
            let diff_provenance = interrogation_error_diff_provenance(&call, xpec_state)?;
            let (diff_from, diff_from_tree_oid, diff_from_tree_oid_abbrev) = diff_provenance
                .map(|provenance| {
                    (
                        Some(provenance.diff_from),
                        Some(provenance.diff_from_tree_oid),
                        Some(provenance.diff_from_tree_oid_abbrev),
                    )
                })
                .unwrap_or((None, None, None));
            let mut answer =
                ParsedAnswer::error(INTERNAL_ERROR_UNPARSABLE.to_string(), err.to_string());
            answer.scope = call.scope.to_vec();
            Ok(InterrogationAnswer {
                answer,
                visible_tree_oid,
                diff_from,
                diff_from_tree_oid,
                diff_from_tree_oid_abbrev,
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

pub(crate) fn initial_q_scope_for_fresh_interrogation(
    root: &std::path::Path,
    expectation: &ResolvedExpectation,
    xpec_state: &mut XpecStateCache,
) -> Result<Vec<String>, String> {
    // Fresh interrogation starts from the expectation's last passing q-scope.
    // If no last pass exists, it starts from full project scope. The actual
    // visible scope is formed later by appending the expectation agent's
    // configured ignore patterns as excluding pathspec items.
    // This scopes the materialized tree and detailed diff, not every prompt
    // signal: prompt rendering also includes an unscoped diff summary, so a
    // changed path outside this scope is still visible enough for the evaluator
    // to report ScopeTooNarrow when it needs the hidden details.
    // `target` is intentionally not an input; target-specific behavior belongs
    // to evaluator prompt rendering.
    let last_pass_q_scope = xpec_state.read_last_pass_q_scope(root, expectation)?;
    Ok(initial_scope_from_last_pass_q_scope(last_pass_q_scope))
}

pub(crate) fn initial_scope_from_last_pass_q_scope(
    last_pass_q_scope: Option<Vec<String>>,
) -> Vec<String> {
    last_pass_q_scope.unwrap_or_else(full_scope)
}

pub(crate) fn q_scope_verification_scope_after_initial_pass(
    runtime: &CheckRuntime<'_>,
    agent: &AgentConfig,
    result: &PolicyInterrogationResult,
    current_scope: &[String],
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Option<Vec<String>>, String> {
    // [YD] `canon check` schedules q-scope verification only after the
    // initial record passed and the full-scope retry follow-up was not used.
    // `ScopeTooNarrow` full-scope retry and q-scope verification share the
    // Interrogation Policy's single follow-up budget. If the retry already
    // consumed that budget, no q-scope verification turn is allowed.
    if !unused_follow_up_can_verify_q_scope(result) {
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

fn unused_follow_up_can_verify_q_scope(result: &PolicyInterrogationResult) -> bool {
    !result.follow_up_used()
        && result.interrogation.record.error.is_none()
        && result.interrogation.record.passed()
}

pub(crate) fn question_scope_suggestion_scope_for_independent_verification(
    runtime: &CheckRuntime<'_>,
    agent: &AgentConfig,
    suggestion: Option<&[String]>,
    current_scope: &[String],
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<Option<Vec<String>>, String> {
    if runtime.evaluator_interrogations_never_hide_files() {
        return Ok(None);
    }
    // Glossary-level q-scope suggestions are evaluator-provided claims. This
    // helper rejects syntactically invalid suggestions before the
    // Interrogation Policy's 25%-smaller verification gate. The response JSON
    // Schema does not require semantically sufficient paths; sufficiency is
    // established only when the independent verification produces an answer.
    // Returning `None` leaves the evaluator's claim unverified; it does not
    // redefine what a q-scope suggestion is.
    let Some(suggestion) = suggestion else {
        return Ok(None);
    };
    let suggested_scope = match sanitize_scope(suggestion) {
        Ok(scope) => scope,
        Err(_) => return Ok(None),
    };
    if !scope_is_within(&suggested_scope, current_scope) {
        return Ok(None);
    }
    if !matches!(
        runtime.visible_tree_oid_if_scope_present(visible_tree_oid_cache, agent, &suggested_scope),
        Ok(Some(_))
    ) {
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

pub(crate) fn narrowed_scope_is_accepted(narrowed: &CheckRecord) -> bool {
    // Acceptance means the q-scope suggestion graduated from evaluator claim
    // to verified reusable q-scope. The initial-pass requirement lives in the
    // verification scheduling gate; once verification runs, any answer result
    // verifies the proposed scope. Error responses do not verify it as
    // reusable, even when they become the final human-review result.
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
        interrogation_error_diff_provenance, narrowed_scope_is_accepted,
        question_scope_suggestion_scope_for_independent_verification,
        suggested_scope_is_at_least_25_percent_smaller, unused_follow_up_can_verify_q_scope,
        InterrogationCall, InterrogationResult, PolicyInterrogationResult,
    };
    use crate::check::core::errors::error_record_from_interrogation_error;
    use crate::check::core::{
        CheckRecord, CheckResult, ResolvedExpectation, ERROR_SCOPE_TOO_NARROW,
    };
    use crate::check::interrogation::state::{CheckRuntime, CheckTreeContext};
    use crate::check::interrogation::InterrogationRequestKind;
    use crate::config_types::{AgentConfig, CheckConfig, AGAINST_TREE_DIFF_FROM};
    use crate::git::{TreeSource, VisibleTreeOidCache};
    use crate::hash::full_scope;
    use crate::staged::StagedWorktreeView;
    use crate::xpec_state::XpecStateCache;
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
    fn initial_scope_uses_last_pass_q_scope_or_full_scope() {
        let q_scope = vec!["src/main.rs".to_string()];

        assert_eq!(
            super::initial_scope_from_last_pass_q_scope(Some(q_scope.clone())),
            q_scope
        );
        assert_eq!(
            super::initial_scope_from_last_pass_q_scope(None),
            full_scope()
        );
    }

    #[test] // xpec: mh
    fn unused_follow_up_q_scope_verification_requires_initial_pass() {
        let pass = test_policy_result(test_record("yes", CheckResult::Pass, None), false);
        let fail = test_policy_result(test_record("no", CheckResult::Fail, None), false);
        let error = test_policy_result(
            test_record(
                ERROR_SCOPE_TOO_NARROW,
                CheckResult::Fail,
                Some(ERROR_SCOPE_TOO_NARROW),
            ),
            false,
        );
        let already_followed_up =
            test_policy_result(test_record("yes", CheckResult::Pass, None), true);

        assert!(unused_follow_up_can_verify_q_scope(&pass));
        assert!(!unused_follow_up_can_verify_q_scope(&fail));
        assert!(!unused_follow_up_can_verify_q_scope(&error));
        assert!(!unused_follow_up_can_verify_q_scope(&already_followed_up));
    }

    #[test] // xpec: mh
    fn narrowed_scope_acceptance_requires_verification_answer() {
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

    #[test] // xpec: mh
    fn disjoint_suggestion_path_is_not_verified_for_narrowing() {
        let root = git_project("disjoint-q-scope-suggestion");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("tests")).unwrap();
        fs::write(root.join("src/one.rs"), "one\n").unwrap();
        fs::write(root.join("src/two.rs"), "two\n").unwrap();
        fs::write(root.join("tests/one.rs"), "test\n").unwrap();
        git(&root, &["add", "src/one.rs", "src/two.rs", "tests/one.rs"]);
        let source = TreeSource::Staged;
        let agent = AgentConfig::default();
        let config = CheckConfig {
            version: 1,
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
        let current_scope = vec!["src".to_string()];
        let suggestion = vec!["tests".to_string()];

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

    #[test] // xpec: mh
    fn no_hide_runtime_never_verifies_q_scope_suggestion() {
        let root = PathBuf::from("/tmp/canon-in-place-policy");
        let agent = AgentConfig::default();
        let config = CheckConfig {
            version: 1,
            agent: agent.clone(),
            expectations: Vec::new(),
        };
        let runtime = CheckRuntime::in_place(&root, &config, false);
        let suggestion = vec!["src".to_string()];
        let mut cache = VisibleTreeOidCache::new();

        let proposed = question_scope_suggestion_scope_for_independent_verification(
            &runtime,
            &agent,
            Some(&suggestion),
            &full_scope(),
            &mut cache,
        )
        .unwrap();

        assert!(proposed.is_none());
    }

    #[test] // xpec: nv,AL
    fn git_backed_interrogation_error_record_preserves_diff_provenance() {
        let root = git_project("interrogation-error-diff-provenance");
        fs::write(root.join("subject.txt"), "subject\n").unwrap();
        git(&root, &["add", "subject.txt"]);
        let source = TreeSource::Staged;
        let agent = AgentConfig::default();
        let config = CheckConfig {
            version: 1,
            agent: agent.clone(),
            expectations: Vec::new(),
        };
        let staged_view = StagedWorktreeView::apply_for_tree_source(&root, source.clone()).unwrap();
        let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
        let tree_context = CheckTreeContext {
            checked_tree_oid: source.tree_oid_for_prompt_diff(&root).unwrap(),
            against_tree_oid: source.tree_oid_for_prompt_diff(&root).unwrap(),
            checked_file_count: visible_tree_oid_cache
                .checked_file_count(&root, &source)
                .unwrap(),
        };
        let against_tree_oid = tree_context.against_tree_oid.clone();
        let runtime =
            CheckRuntime::materialized(&root, &staged_view, &source, tree_context, &config, false);
        let expectation = test_expectation(&agent);
        let scope = full_scope();
        let call = InterrogationCall {
            runtime: &runtime,
            expectation: &expectation,
            scope: &scope,
            request_kind: InterrogationRequestKind::Initial,
            progress: None,
        };
        let mut xpec_state = XpecStateCache::default();

        let diff_provenance = interrogation_error_diff_provenance(&call, &mut xpec_state).unwrap();
        let record = error_record_from_interrogation_error(
            &runtime,
            &agent,
            &expectation,
            &scope,
            "turn failed",
            diff_provenance,
            &mut visible_tree_oid_cache,
        )
        .unwrap();

        assert_eq!(record.diff_from.as_deref(), Some(AGAINST_TREE_DIFF_FROM));
        assert_eq!(
            record.diff_from_tree_oid.as_deref(),
            Some(against_tree_oid.as_str())
        );
        assert_eq!(
            record.diff_from_tree_oid_abbrev.as_deref(),
            Some(
                crate::git::abbreviate_git_oid(&root, &against_tree_oid)
                    .unwrap()
                    .as_str()
            )
        );
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
            to: crate::config_types::ExpectationTo::Agent,
            question: Some("question".to_string()),
            expected_answer: Some("yes".to_string()),
            observed: observed.to_string(),
            error: error.map(str::to_string),
            evidence: "evidence".to_string(),
            scope: full_scope(),
            question_scope_suggestion: None,
            visible_tree_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
            id: "expectation".to_string(),
            display_id: "e".to_string(),
        }
    }

    fn test_expectation(agent: &AgentConfig) -> ResolvedExpectation {
        ResolvedExpectation {
            number: 0,
            id: "expectation-id".to_string(),
            display_id: "e".to_string(),
            to: crate::config_types::ExpectationTo::Agent,
            rank: 0,
            question: "Does this fail technically?".to_string(),
            expected_answer: "yes".to_string(),
            question_context: String::new(),
            diff_from: AGAINST_TREE_DIFF_FROM.to_string(),
            target: None,
            question_answer_only: false,
            agent: agent.clone(),
            cooldown: None,
        }
    }

    fn test_policy_result(record: CheckRecord, follow_up_used: bool) -> PolicyInterrogationResult {
        PolicyInterrogationResult::new(
            InterrogationResult {
                record,
                turn_usage: None,
                context_compacted: false,
                stop_after_current_expectation: false,
                interrupted: false,
            },
            follow_up_used,
        )
    }
}
