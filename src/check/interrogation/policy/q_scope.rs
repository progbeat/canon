use crate::check::core::InterrogationResult;
use crate::check::interrogation::scope_narrowing_log_fields;
use crate::check::interrogation::state::CheckRuntime;
use crate::check::q_scope::validated_narrower_q_scope_suggestion;
use crate::config_types::AgentConfig;
use crate::git::VisibleTreeOidCache;
use crate::logs::DiagnosticLogWriter;

pub(crate) fn interrogation_has_passing_answer_for_q_scope_verification(
    result: &InterrogationResult,
) -> bool {
    // [kg] Verification depends only on the current passing response and its
    // suggestion. A full-scope retry is mutually exclusive with verification.
    result.output.error.is_none() && result.output.passed()
}

pub(crate) fn q_scope_suggestion_scope_for_independent_verification(
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
    // Returning `None` rejects only the proposed *next* narrowing decision. It
    // does not invalidate an answer already produced under the enforced
    // current q-scope, which is stored separately from qScopeSuggestion.
    let Some(suggestion) = suggestion else {
        return Ok(None);
    };
    let source = runtime
        .tree_source()
        .ok_or("q-scope verification requires a Git tree source".to_string())?;
    validated_narrower_q_scope_suggestion(
        runtime.root,
        source,
        agent,
        suggestion,
        current_scope,
        visible_tree_oid_cache,
    )
}

pub(crate) fn write_scope_narrowing_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    id: Option<&str>,
    enforced_scope: &[String],
    record_scope: &[String],
    accepted: bool,
) -> Result<(), String> {
    let Some(writer) = diagnostic_log.as_deref_mut() else {
        return Ok(());
    };
    writer
        .emit_event(
            "info",
            "scope.narrowing",
            &scope_narrowing_log_fields(id, enforced_scope, record_scope, accepted),
        )
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::core::{CheckRecord, CheckResult, ERROR_SCOPE_TOO_NARROW};
    use crate::check::interrogation::state::CheckTreeContext;
    use crate::config_types::CheckConfig;
    use crate::git::TreeSource;
    use crate::hash::full_scope;
    use crate::materialization::TreeMaterializer;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test] // xpec: qv
    fn q_scope_verification_requires_a_passing_response() {
        let pass = test_policy_result(test_record("yes", CheckResult::Pass, None));
        let fail = test_policy_result(test_record("no", CheckResult::Fail, None));
        let error = test_policy_result(test_record(
            ERROR_SCOPE_TOO_NARROW,
            CheckResult::Fail,
            Some(ERROR_SCOPE_TOO_NARROW),
        ));

        assert!(interrogation_has_passing_answer_for_q_scope_verification(
            &pass
        ));
        assert!(!interrogation_has_passing_answer_for_q_scope_verification(
            &fail
        ));
        assert!(!interrogation_has_passing_answer_for_q_scope_verification(
            &error
        ));
    }

    #[test] // xpec: qv
    fn fully_absent_suggestion_is_not_verified_for_narrowing() {
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
        let tree_materializer =
            TreeMaterializer::apply_for_tree_source(&root, source.clone()).unwrap();
        let mut cache = VisibleTreeOidCache::new();
        let tree_context = CheckTreeContext {
            checked_tree_oid: source.tree_oid_for_prompt_diff(&root).unwrap(),
            against_tree_oid: source.tree_oid_for_prompt_diff(&root).unwrap(),
            head_tree_oid: None,
            explicit_diff_from_tree_oids: Default::default(),
            checked_file_count: cache.checked_file_count(&root, &source).unwrap(),
            prompt_git_environment: Vec::new(),
        };
        let runtime = CheckRuntime::materialized(
            &root,
            &tree_materializer,
            &source,
            tree_context,
            &config,
            false,
        );
        let current_scope = vec![".".to_string()];
        let suggestion = vec![
            "src/missing.rs".to_string(),
            "src/also-missing.rs".to_string(),
        ];

        let proposed = q_scope_suggestion_scope_for_independent_verification(
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

    #[test] // xpec: qv
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
        let tree_materializer =
            TreeMaterializer::apply_for_tree_source(&root, source.clone()).unwrap();
        let mut cache = VisibleTreeOidCache::new();
        let tree_context = CheckTreeContext {
            checked_tree_oid: source.tree_oid_for_prompt_diff(&root).unwrap(),
            against_tree_oid: source.tree_oid_for_prompt_diff(&root).unwrap(),
            head_tree_oid: None,
            explicit_diff_from_tree_oids: Default::default(),
            checked_file_count: cache.checked_file_count(&root, &source).unwrap(),
            prompt_git_environment: Vec::new(),
        };
        let runtime = CheckRuntime::materialized(
            &root,
            &tree_materializer,
            &source,
            tree_context,
            &config,
            false,
        );
        let current_scope = vec!["src".to_string()];
        let suggestion = vec!["tests".to_string()];

        let proposed = q_scope_suggestion_scope_for_independent_verification(
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

    #[test] // xpec: qv
    fn no_hide_runtime_never_verifies_q_scope_suggestion() {
        let root = PathBuf::from("/tmp/canon-in-place-policy");
        let agent = AgentConfig::default();
        let config = CheckConfig {
            version: 1,
            agent: agent.clone(),
            expectations: Vec::new(),
        };
        let runtime = CheckRuntime::in_place(&root, &config, true);
        let suggestion = vec!["src".to_string()];
        let mut cache = VisibleTreeOidCache::new();

        let proposed = q_scope_suggestion_scope_for_independent_verification(
            &runtime,
            &agent,
            Some(&suggestion),
            &full_scope(),
            &mut cache,
        )
        .unwrap();

        assert!(proposed.is_none());
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
        // xpec: qv
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
            result,
            to: crate::config_types::ExpectationTo::Agent,
            question: Some("question".to_string()),
            expected_answer: Some("yes".to_string()),
            observed: observed.to_string(),
            error: error.map(str::to_string),
            evidence: Some("evidence".to_string()),
            scope: full_scope(),
            q_scope_suggestion: None,
            visible_tree_oid: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
            id: "expectation".to_string(),
            display_id: "e".to_string(),
        }
    }

    fn test_policy_result(record: CheckRecord) -> InterrogationResult {
        InterrogationResult::new(record, false, false)
    }
}
