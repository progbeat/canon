use super::*;
use crate::check::core::{CheckRecord, CheckResult};
use crate::check::ExpectationIdentity;
use crate::config_types::{AgentConfig, Cooldown, ExpectationTarget, QScope};
use crate::hash::full_scope;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};

#[test] // xpec: m,2gZ
fn default_runs_reuse_cached_results() {
    let root = git_project("default-reuses-cache");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn demo() {}\n").unwrap();
    git(&root, &["add", "src/lib.rs"]);

    let expectation = test_expectation();
    let source = TreeSource::Staged;
    let scope = vec!["src/lib.rs".to_string()];
    let checked_tree_oid = source.tree_oid_for_prompt_diff(&root).unwrap();
    let visible_tree_oid = VisibleTreeOidCache::new()
        .visible_tree_oid(&root, &source, &expectation.agent, &scope)
        .unwrap();
    write_last_result_for_test(
        &root,
        &expectation,
        &checked_tree_oid,
        &test_record(&expectation, &scope, "yes", visible_tree_oid),
    );

    let work = cache_filtered_work(&root, &source, expectation);
    assert_eq!(work.reused_non_selected_results.len(), 1);
    assert!(work.selected_evaluation_queue.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: UH
fn auto_full_scope_pass_remains_cacheable() {
    let root = git_project("auto-full-scope-cache");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn demo() {}\n").unwrap();
    git(&root, &["add", "src/lib.rs"]);

    let expectation = test_expectation();
    let source = TreeSource::Staged;
    let scope = full_scope();
    let checked_tree_oid = source.tree_oid_for_prompt_diff(&root).unwrap();
    let visible_tree_oid = VisibleTreeOidCache::new()
        .visible_tree_oid(&root, &source, &expectation.agent, &scope)
        .unwrap();
    write_last_result_for_test(
        &root,
        &expectation,
        &checked_tree_oid,
        &test_record(&expectation, &scope, "yes", visible_tree_oid),
    );

    let work = cache_filtered_work(&root, &source, expectation);

    assert_eq!(work.reused_non_selected_results.len(), 1);
    assert!(work.selected_evaluation_queue.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: UH
fn fixed_full_scope_pass_remains_cacheable() {
    let root = git_project("fixed-full-scope-cache");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn demo() {}\n").unwrap();
    git(&root, &["add", "src/lib.rs"]);

    let mut expectation = test_expectation();
    expectation.q_scope = QScope::Paths(full_scope());
    let source = TreeSource::Staged;
    let scope = full_scope();
    let checked_tree_oid = source.tree_oid_for_prompt_diff(&root).unwrap();
    let visible_tree_oid = VisibleTreeOidCache::new()
        .visible_tree_oid(&root, &source, &expectation.agent, &scope)
        .unwrap();
    write_last_result_for_test(
        &root,
        &expectation,
        &checked_tree_oid,
        &test_record(&expectation, &scope, "yes", visible_tree_oid),
    );

    let work = cache_filtered_work(&root, &source, expectation);

    assert_eq!(work.reused_non_selected_results.len(), 1);
    assert!(work.selected_evaluation_queue.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: 2gZ,H9
fn selector_mode_forces_evaluation_despite_cached_results() {
    let root = git_project("selector-cache-continues");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn demo() {}\n").unwrap();
    git(&root, &["add", "src/lib.rs"]);

    let cached_expectation = test_expectation();
    let uncached_expectation = test_expectation_with_identity("def456", "d");
    let source = TreeSource::Staged;
    let scope = full_scope();
    let checked_tree_oid = source.tree_oid_for_prompt_diff(&root).unwrap();
    let visible_tree_oid = VisibleTreeOidCache::new()
        .visible_tree_oid(&root, &source, &cached_expectation.agent, &scope)
        .unwrap();
    write_last_result_for_test(
        &root,
        &cached_expectation,
        &checked_tree_oid,
        &test_record(&cached_expectation, &scope, "no", visible_tree_oid),
    );

    let work = cache_filtered_work_with_mode(
        &root,
        &source,
        vec![uncached_expectation, cached_expectation],
        true,
    );
    assert!(work.reused_non_selected_results.is_empty());
    assert_eq!(work.selected_evaluation_queue.len(), 2);
    assert_eq!(
        work.selected_evaluation_queue[0].configured_id(),
        Some("abc123")
    );
    assert_eq!(
        work.selected_evaluation_queue[1].configured_id(),
        Some("def456")
    );
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: 2gZ,m
fn same_tree_fail_history_is_not_a_cached_result() {
    let root = git_project("default-fail-history-does-not-cache");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn demo() {}\n").unwrap();
    git(&root, &["add", "src/lib.rs"]);

    let cached_expectation = test_expectation();
    let uncached_expectation = test_expectation_with_identity("def456", "d");
    let source = TreeSource::Staged;
    let scope = full_scope();
    let checked_tree_oid = source.tree_oid_for_prompt_diff(&root).unwrap();
    let visible_tree_oid = VisibleTreeOidCache::new()
        .visible_tree_oid(&root, &source, &cached_expectation.agent, &scope)
        .unwrap();
    write_last_result_for_test(
        &root,
        &cached_expectation,
        &checked_tree_oid,
        &test_record(&cached_expectation, &scope, "no", visible_tree_oid),
    );

    let work = cache_filtered_work_with_mode(
        &root,
        &source,
        vec![cached_expectation, uncached_expectation],
        false,
    );
    assert!(work.reused_non_selected_results.is_empty());
    assert_eq!(work.selected_evaluation_queue.len(), 2);
    let _ = fs::remove_dir_all(root);
}

fn cache_filtered_work(
    root: &Path,
    source: &TreeSource,
    expectation: ResolvedExpectation,
) -> GitBackedCacheFilteredCheckWork {
    cache_filtered_work_with_mode(root, source, vec![expectation], false)
}

fn cache_filtered_work_with_mode(
    root: &Path,
    source: &TreeSource,
    expectations: Vec<ResolvedExpectation>,
    selectors_provided: bool,
) -> GitBackedCacheFilteredCheckWork {
    let mut xpec_state = XpecStateCache::default();
    let identities = expectations
        .iter()
        .map(|expectation| ExpectationIdentity {
            id: expectation.require_configured_id().unwrap().to_string(),
            display_id: expectation.display_id.clone(),
        })
        .collect::<Vec<_>>();
    xpec_state
        .retain_only_current_configuration(root, &identities)
        .unwrap();
    let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
    let mut diagnostic_log = None;
    let checked_tree_oid = source.tree_oid_for_prompt_diff(root).unwrap();
    select_and_order_git_backed_expectations(
        GitBackedCacheFilterContext {
            root,
            source,
            checked_tree_oid: &checked_tree_oid,
            xpec_state: &mut xpec_state,
            visible_tree_oid_cache: &mut visible_tree_oid_cache,
            diagnostic_log: &mut diagnostic_log,
        },
        &CheckOptions {
            candidate_expectations: expectations,
            selectors_provided,
            keep_going: false,
        },
        2,
    )
    .unwrap()
}

fn test_expectation() -> ResolvedExpectation {
    test_expectation_with_identity("abc123", "a")
}

fn test_expectation_with_identity(id: &str, display_id: &str) -> ResolvedExpectation {
    ResolvedExpectation {
        kind: crate::check::core::ResolvedExpectationKind::Configured { id: id.to_string() },
        display_id: display_id.to_string(),
        to: crate::config_types::ExpectationTo::Agent,
        rank: 0,
        question: format!("Does {id} pass?"),
        expected_answer: "yes".to_string(),
        question_context: String::new(),
        diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
        target: Option::<ExpectationTarget>::None,
        agent: AgentConfig::default(),
        cooldown: Option::<Cooldown>::None,
        q_scope: Default::default(),
    }
}

fn test_record(
    expectation: &ResolvedExpectation,
    scope: &[String],
    observed: &str,
    visible_tree_oid: String,
) -> CheckRecord {
    CheckRecord {
        timestamp: crate::time::format_record_timestamp(1),
        result: CheckResult::from_expected_answer(expectation.expected_answer(), observed),
        to: crate::config_types::ExpectationTo::Agent,
        question: Some(expectation.question.clone()),
        expected_answer: Some(expectation.expected_answer().to_string()),
        observed: observed.to_string(),
        error: None,
        evidence: Some("evidence".to_string()),
        scope: scope.to_vec(),
        q_scope_suggestion: Some(scope.to_vec()),
        visible_tree_oid: Some(visible_tree_oid),
        diff_from: Some(crate::config_types::DEFAULT_DIFF_FROM.to_string()),
        diff_from_tree_oid: Some("1234567890abcdef1234567890abcdef12345678".to_string()),
        diff_from_tree_oid_abbrev: Some("1234567".to_string()),
        id: expectation.require_configured_id().unwrap().to_string(),
        display_id: expectation.display_id.clone(),
    }
}

fn write_last_result_for_test(
    root: &Path,
    expectation: &ResolvedExpectation,
    checked_tree_oid: &str,
    record: &CheckRecord,
) {
    let identity = ExpectationIdentity {
        id: expectation.require_configured_id().unwrap().to_string(),
        display_id: expectation.display_id.clone(),
    };
    let mut xpec_state = XpecStateCache::default();
    xpec_state
        .retain_only_current_configuration(root, &[identity])
        .unwrap();
    xpec_state
        .write_last_result_for_record(root, Some(checked_tree_oid), expectation, record)
        .unwrap();
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
            "canon-cache-select-{}-{}-{}",
            name,
            process::id(),
            unique
        ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    root
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    // xpec: m,2gZ
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}
