use super::gate_command_result;
use crate::check::{
    expectation_identities, is_missing_default_config_error, load_check_config,
    select_expectations_with_identities, CHECK_PATH,
};
use crate::cli::CommandError;
use crate::config_types::CheckConfig;
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::repo_inspection::RepoInspectionCache;
use crate::time::unix_timestamp;
use crate::xpec_state::XpecStateCache;
use cache::{ComparisonTree, ComparisonTrees};
use std::path::Path;

mod cache;

pub(super) fn count(
    root: &Path,
    repo_cache: &mut RepoInspectionCache,
    baseline_source: &TreeSource,
    staged_source: &TreeSource,
) -> Result<usize, CommandError> {
    // Staged canon edits are handled by the gate decision after this count is known.
    let config = match load_check_config(repo_cache, root, Path::new(CHECK_PATH), baseline_source) {
        Ok(config) => config,
        Err(err) if is_missing_default_config_error(&err) => return Ok(0),
        Err(err) => return gate_command_result(Err(err)),
    };
    let mut visible_tree_oid_cache =
        VisibleTreeOidCache::with_repo_inspection_cache(repo_cache.clone());
    // [g2] This owner and its memo tables remain in memory. Its IO methods
    // access bounded cross-invocation Last Results and gate history; they never
    // serialize the invocation-local cache itself.
    let mut xpec_state = XpecStateCache::default();
    gate_command_result(count_with_config(
        root,
        &config,
        baseline_source,
        staged_source,
        &mut xpec_state,
        &mut visible_tree_oid_cache,
    ))
}

fn count_with_config(
    root: &Path,
    config: &CheckConfig,
    baseline_source: &TreeSource,
    staged_source: &TreeSource,
    xpec_state: &mut XpecStateCache,
    visible_tree_oid_cache: &mut VisibleTreeOidCache,
) -> Result<usize, String> {
    let identities = expectation_identities(config)?;
    let selected_expectations = select_expectations_with_identities(config, &identities, &[])?;
    let baseline_tree_oid = baseline_source.resolved_tree_oid()?;
    let staged_tree_oid = staged_source.resolved_tree_oid()?;
    let trees = ComparisonTrees {
        previous: ComparisonTree {
            source: baseline_source,
            tree_oid: baseline_tree_oid,
        },
        current: ComparisonTree {
            source: staged_source,
            tree_oid: staged_tree_oid,
        },
    };
    let now = unix_timestamp()?;
    cache::count(
        root,
        &selected_expectations,
        trees,
        xpec_state,
        visible_tree_oid_cache,
        now,
    )
}

// [Pi,c0] These tests assert the regression component entry point, which stays
// visible only to its parent gate component. Keep them in the implementation
// file instead of widening that interface solely for external tests.
#[cfg(test)]
mod tests {
    use super::count;
    use crate::check::{
        expectation_identities, select_expectations_with_identities, CheckRecord, CheckResult,
        ExpectationIdentity, ResolvedExpectation,
    };
    use crate::config_types::{AgentConfig, CheckConfig, Expectation, ExpectationTo};
    use crate::git::{TreeSource, VisibleTreeOidCache, DEFAULT_AGAINST_TREE_ARG};
    use crate::hash::full_scope;
    use crate::repo_inspection::RepoInspectionCache;
    use crate::time::format_record_timestamp;
    use crate::xpec_state::XpecStateCache;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    #[test] // xpec: KD,cw
    fn unborn_repository_without_config_has_no_gate_regressions() -> Result<(), String> {
        let root = test_root("unborn");
        fs::create_dir_all(&root)
            .map_err(|err| format!("failed to create test repository: {err}"))?;
        run_git(&root, &["init", "--quiet"])?;

        let mut repo_cache = RepoInspectionCache::new();
        let baseline = repo_cache.resolve_default_against_tree(&root, DEFAULT_AGAINST_TREE_ARG)?;
        let staged = repo_cache.resolve_tree_to_oid_source(&root, ":staged", "--tree")?;
        let regressions =
            count(&root, &mut repo_cache, &baseline, &staged).map_err(|err| err.to_string())?;

        assert_eq!(regressions, 0);
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test] // xpec: W4,cw
    fn same_tree_pass_clears_an_expectation_regression() -> Result<(), String> {
        let root = committed_project("same-tree-pass")?;
        fs::write(root.join("README.md"), "staged\n")
            .map_err(|err| format!("failed to write staged file: {err}"))?;
        run_git(&root, &["add", "README.md"])?;

        let config = check_config();
        let expectation = resolved_expectation(&config)?;
        let identity = ExpectationIdentity {
            id: expectation.require_configured_id()?.to_string(),
            display_id: expectation.display_id.clone(),
        };
        let mut state = XpecStateCache::default();
        state.retain_only_current_configuration(&root, &[identity])?;

        let baseline = RepoInspectionCache::new()
            .resolve_default_against_tree(&root, DEFAULT_AGAINST_TREE_ARG)?;
        let mut tree_cache = VisibleTreeOidCache::new();
        let staged = TreeSource::Staged;
        let scope = full_scope();
        let baseline_oid = baseline.tree_oid_for_prompt_diff(&root)?;
        let staged_oid = staged.tree_oid_for_prompt_diff(&root)?;
        let baseline_visible_oid =
            tree_cache.visible_tree_oid(&root, &baseline, &expectation.agent, &scope)?;
        let staged_visible_oid =
            tree_cache.visible_tree_oid(&root, &staged, &expectation.agent, &scope)?;

        state.write_last_result_for_record(
            &root,
            Some(&baseline_oid),
            &expectation,
            &record(&expectation, "yes", baseline_visible_oid, 1),
        )?;
        state.write_last_result_for_record(
            &root,
            Some(&staged_oid),
            &expectation,
            &record(&expectation, "no", staged_visible_oid.clone(), 2),
        )?;

        let mut gate_cache = RepoInspectionCache::new();
        let gate_baseline =
            gate_cache.resolve_default_against_tree(&root, DEFAULT_AGAINST_TREE_ARG)?;
        let gate_staged = gate_cache.resolve_tree_to_oid_source(&root, ":staged", "--tree")?;
        assert_eq!(
            count(&root, &mut gate_cache, &gate_baseline, &gate_staged)
                .map_err(|err| err.to_string())?,
            1
        );

        state.write_last_result_for_record(
            &root,
            Some(&staged_oid),
            &expectation,
            &record(&expectation, "yes", staged_visible_oid, 3),
        )?;

        let mut gate_cache = RepoInspectionCache::new();
        let gate_baseline =
            gate_cache.resolve_default_against_tree(&root, DEFAULT_AGAINST_TREE_ARG)?;
        let gate_staged = gate_cache.resolve_tree_to_oid_source(&root, ":staged", "--tree")?;
        let unresolved_pass_to_fail_regressions =
            count(&root, &mut gate_cache, &gate_baseline, &gate_staged)
                .map_err(|err| err.to_string())?;
        assert_eq!(unresolved_pass_to_fail_regressions, 0);
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    fn record(
        expectation: &ResolvedExpectation,
        observed: &str,
        visible_tree_oid: String,
        timestamp: u64,
    ) -> CheckRecord {
        CheckRecord {
            timestamp: format_record_timestamp(timestamp),
            result: CheckResult::from_expected_answer(expectation.expected_answer(), observed),
            to: ExpectationTo::Agent,
            question: Some(expectation.question.clone()),
            expected_answer: Some(expectation.expected_answer().to_string()),
            observed: observed.to_string(),
            error: None,
            evidence: Some("test evidence".to_string()),
            scope: full_scope(),
            q_scope_suggestion: None,
            visible_tree_oid: Some(visible_tree_oid),
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
            id: expectation.require_configured_id().unwrap().to_string(),
            display_id: expectation.display_id.clone(),
        }
    }

    fn resolved_expectation(config: &CheckConfig) -> Result<ResolvedExpectation, String> {
        let identities = expectation_identities(config)?;
        select_expectations_with_identities(config, &identities, &[])?
            .into_iter()
            .next()
            .ok_or_else(|| "test config did not resolve an expectation".to_string())
    }

    fn check_config() -> CheckConfig {
        let agent = AgentConfig::default();
        CheckConfig {
            version: 1,
            agent: agent.clone(),
            expectations: vec![Expectation {
                to: ExpectationTo::Agent,
                rank: 0,
                q: QUESTION.to_string(),
                a: "yes".to_string(),
                question_context: String::new(),
                diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
                target: None,
                agent,
                cooldown: None,
                q_scope: Default::default(),
                in_place_compatibility: Default::default(),
            }],
        }
    }

    fn committed_project(name: &str) -> Result<PathBuf, String> {
        let root = test_root(name);
        fs::create_dir_all(&root)
            .map_err(|err| format!("failed to create test repository: {err}"))?;
        for args in [
            &["init", "--quiet"][..],
            &["config", "user.name", "Canon Test"][..],
            &["config", "user.email", "canon@example.invalid"][..],
        ] {
            run_git(&root, args)?;
        }
        fs::write(root.join("README.md"), "baseline\n")
            .map_err(|err| format!("failed to write baseline file: {err}"))?;
        fs::create_dir_all(root.join(".canon"))
            .map_err(|err| format!("failed to create canon directory: {err}"))?;
        fs::write(
            root.join(".canon/check.yml"),
            format!("presets:\n  default: {{}}\nxpecs:\n  - q: {QUESTION}\n    a: yes\n"),
        )
        .map_err(|err| format!("failed to write canon config: {err}"))?;
        run_git(&root, &["add", "README.md", ".canon/check.yml"])?;
        run_git(&root, &["commit", "--quiet", "-m", "baseline"])?;
        Ok(root)
    }

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "canon-gate-{name}-{}-{:016x}",
            std::process::id(),
            getrandom::u64().unwrap_or(0)
        ))
    }

    const QUESTION: &str = "Does the staged tree pass?";

    fn run_git(root: &Path, args: &[&str]) -> Result<(), String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .map_err(|err| format!("failed to run git {}: {err}", args.join(" ")))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }
}
