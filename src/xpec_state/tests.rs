use super::*;
use crate::check::{CheckRecord, CheckResult, SelectedExpectation};
use crate::config_types::{AgentConfig, ExpectationTarget};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::hash::full_scope;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn last_result_files_use_status_dependent_fields_and_last_json_follows_error() {
    let root = git_project("last-result-status-fields");
    let expectation = test_expectation();
    let mut cache = XpecStateCache::default();
    let scope = full_scope();

    let pass = test_record(&expectation, &scope, "yes", None);
    cache
        .write_last_result_for_record(&root, "checked-tree", &expectation, &pass)
        .unwrap();
    let pass_json = read_json(&root, &expectation.id, "last-pass.json");
    assert_eq!(pass_json["status"], "pass");
    assert_eq!(
        pass_json["response"]["qScopeSuggestion"],
        serde_json::json!(["."])
    );
    assert_eq!(pass_json["checkedTreeOid"], "checked-tree");
    assert_eq!(pass_json["visibleTreeOid"], "visible-tree");

    let fail = test_record(&expectation, &scope, "no", None);
    cache
        .write_last_result_for_record(&root, "checked-tree", &expectation, &fail)
        .unwrap();
    let fail_json = read_json(&root, &expectation.id, "last-fail.json");
    assert_eq!(fail_json["status"], "fail");
    assert!(fail_json.get("checkedTreeOid").is_none());
    assert_eq!(fail_json["visibleTreeOid"], "visible-tree");

    let error = test_record(&expectation, &scope, "unparsable", Some("unparsable"));
    cache
        .write_last_result_for_record(&root, "checked-tree", &expectation, &error)
        .unwrap();
    let error_json = read_json(&root, &expectation.id, "last-error.json");
    assert_eq!(error_json["status"], "error");
    assert!(error_json.get("checkedTreeOid").is_none());
    assert!(error_json.get("visibleTreeOid").is_none());

    let last_json = read_json(&root, &expectation.id, "last.json");
    assert_eq!(last_json, error_json);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn last_error_is_not_a_cached_result() {
    let root = git_project("last-error-not-cached");
    let expectation = test_expectation();
    let mut cache = XpecStateCache::default();
    let scope = full_scope();

    let error = test_record(&expectation, &scope, "unparsable", Some("unparsable"));
    cache
        .write_last_result_for_record(&root, "checked-tree", &expectation, &error)
        .unwrap();

    let hit = cached_last_result_for_expectation(
        &root,
        &TreeSource::Staged,
        &expectation,
        &mut cache,
        &mut VisibleTreeOidCache::new(),
        CachedLastResultLookup {
            now: 2,
            include_same_tree: true,
            include_cooldown: true,
        },
    )
    .unwrap();

    assert!(hit.is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn last_result_without_usable_answer_uses_error_status() {
    let root = git_project("last-result-unusable-answer");
    let expectation = test_expectation();
    let mut cache = XpecStateCache::default();
    let scope = full_scope();

    let invalid_answer = test_record(&expectation, &scope, "not usable", None);
    cache
        .write_last_result_for_record(&root, "checked-tree", &expectation, &invalid_answer)
        .unwrap();

    let error_json = read_json(&root, &expectation.id, "last-error.json");
    assert_eq!(error_json["status"], "error");
    assert_eq!(error_json["response"]["error"], "unparsable");
    assert!(error_json.get("checkedTreeOid").is_none());
    assert!(error_json.get("visibleTreeOid").is_none());
    assert!(!last_result_path(&root, &expectation.id, "last-fail.json").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn last_error_keeps_response_suggestion_separate_from_persisted_q_scope() {
    let root = git_project("last-error-suggestion-separate");
    let expectation = test_expectation();
    let mut cache = XpecStateCache::default();
    let scope = vec!["src/too-narrow.rs".to_string()];
    let suggestion = vec!["src/needed.rs".to_string()];
    let mut error = test_record(
        &expectation,
        &scope,
        "ScopeTooNarrow",
        Some("ScopeTooNarrow"),
    );
    error.question_scope_suggestion = Some(suggestion.clone());

    cache
        .write_last_result_for_record(&root, "checked-tree", &expectation, &error)
        .unwrap();

    let error_json = read_json(&root, &expectation.id, "last-error.json");
    assert_eq!(error_json["qScope"], json!(scope));
    assert_eq!(
        error_json["response"]["qScopeSuggestion"],
        json!(suggestion)
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn last_pass_keeps_response_suggestion_separate_from_persisted_q_scope() {
    let root = git_project("last-pass-suggestion-separate");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/narrow.rs"), "narrow\n").unwrap();
    fs::write(root.join("src/extra.rs"), "extra\n").unwrap();
    git(&root, &["add", "src/narrow.rs", "src/extra.rs"]);

    let expectation = test_expectation();
    let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
    let persisted_scope = vec!["src/narrow.rs".to_string()];
    let mut record = test_record(&expectation, &persisted_scope, "yes", None);
    record.visible_tree_oid = VisibleTreeOidCache::new()
        .visible_tree_oid(
            &root,
            &TreeSource::Staged,
            &expectation.agent,
            &persisted_scope,
        )
        .unwrap();
    record.question_scope_suggestion = Some(full_scope());

    XpecStateCache::default()
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &record)
        .unwrap();

    let pass_json = read_json(&root, &expectation.id, "last-pass.json");
    assert_eq!(pass_json["qScope"], json!(persisted_scope));
    assert_eq!(pass_json["response"]["qScopeSuggestion"], json!(["."]));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn same_tree_pass_reuses_last_pass_when_only_hidden_files_change() {
    let root = git_project("same-tree-pass-hidden-change");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a.rs"), "a\n").unwrap();
    fs::write(root.join("src/b.rs"), "b\n").unwrap();
    git(&root, &["add", "src/a.rs", "src/b.rs"]);

    let mut expectation = test_expectation();
    expectation.target = Some(ExpectationTarget::Diff);
    let mut cache = XpecStateCache::default();
    let q_scope = vec!["src/a.rs".to_string()];
    let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
    let mut record = test_record(&expectation, &q_scope, "yes", None);
    record.visible_tree_oid = VisibleTreeOidCache::new()
        .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &q_scope)
        .unwrap();
    record.question_scope_suggestion = Some(full_scope());
    cache
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &record)
        .unwrap();

    fs::write(root.join("src/b.rs"), "changed\n").unwrap();
    git(&root, &["add", "src/b.rs"]);
    let refreshed_checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
    assert_ne!(checked_tree_oid, refreshed_checked_tree_oid);

    let hit = cached_last_result_for_expectation(
        &root,
        &TreeSource::Staged,
        &expectation,
        &mut cache,
        &mut VisibleTreeOidCache::new(),
        CachedLastResultLookup {
            now: 2,
            include_same_tree: true,
            include_cooldown: true,
        },
    )
    .unwrap()
    .unwrap();

    assert_eq!(hit.status, CachedResultStatus::Pass);
    assert_eq!(hit.kind, CachedLastResultKind::SameTree);
    assert_eq!(hit.result.q_scope, q_scope);

    let hit = refresh_reused_same_tree_last_result(
        &root,
        &TreeSource::Staged,
        &expectation,
        &mut cache,
        hit,
    )
    .unwrap();

    assert_eq!(
        hit.result.checked_tree_oid.as_deref(),
        Some(refreshed_checked_tree_oid.as_str())
    );
    let pass_json = read_json(&root, &expectation.id, "last-pass.json");
    assert_eq!(pass_json["checkedTreeOid"], refreshed_checked_tree_oid);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn same_tree_result_ignores_diff_from_prompt_direction() {
    let root = git_project("same-tree-ignores-diff-from");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a.rs"), "a\n").unwrap();
    git(&root, &["add", "src/a.rs"]);

    let mut expectation = test_expectation();
    expectation.target = Some(ExpectationTarget::Diff);
    let mut cache = XpecStateCache::default();
    let scope = full_scope();
    let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
    let mut record = test_record(&expectation, &scope, "yes", None);
    record.visible_tree_oid = VisibleTreeOidCache::new()
        .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &scope)
        .unwrap();
    cache
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &record)
        .unwrap();

    expectation.diff_from = crate::config_types::AGAINST_TREE_DIFF_FROM.to_string();
    let hit = cached_last_result_for_expectation(
        &root,
        &TreeSource::Staged,
        &expectation,
        &mut cache,
        &mut VisibleTreeOidCache::new(),
        CachedLastResultLookup {
            now: 2,
            include_same_tree: true,
            include_cooldown: true,
        },
    )
    .unwrap()
    .unwrap();

    assert_eq!(hit.status, CachedResultStatus::Pass);
    assert_eq!(hit.kind, CachedLastResultKind::SameTree);
    assert_eq!(hit.result.answer(), Some("yes"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn new_answer_status_removes_stale_opposite_answer_result() {
    let root = git_project("new-answer-removes-stale-opposite");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a.rs"), "a\n").unwrap();
    fs::write(root.join("src/b.rs"), "b\n").unwrap();
    git(&root, &["add", "src/a.rs", "src/b.rs"]);

    let expectation = test_expectation();
    let mut cache = XpecStateCache::default();
    let fail_scope = vec!["src/a.rs".to_string()];
    let pass_scope = full_scope();
    let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
    let mut oid_cache = VisibleTreeOidCache::new();

    let mut fail = test_record(&expectation, &fail_scope, "no", None);
    fail.visible_tree_oid = oid_cache
        .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &fail_scope)
        .unwrap();
    cache
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &fail)
        .unwrap();

    let mut pass = test_record(&expectation, &pass_scope, "yes", None);
    pass.visible_tree_oid = oid_cache
        .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &pass_scope)
        .unwrap();
    cache
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &pass)
        .unwrap();

    assert!(!last_result_path(&root, &expectation.id, "last-fail.json").exists());
    let hit = cached_last_result_for_expectation(
        &root,
        &TreeSource::Staged,
        &expectation,
        &mut cache,
        &mut VisibleTreeOidCache::new(),
        CachedLastResultLookup {
            now: 2,
            include_same_tree: true,
            include_cooldown: true,
        },
    )
    .unwrap()
    .unwrap();

    assert_eq!(hit.status, CachedResultStatus::Pass);
    assert_eq!(hit.kind, CachedLastResultKind::SameTree);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stored_q_scope_uses_latest_result() {
    let root = git_project("stored-q-scope-latest-result");
    let expectation = test_expectation();
    let mut cache = XpecStateCache::default();
    let pass_scope = vec!["src/pass.rs".to_string()];
    let fail_scope = vec!["src/fail.rs".to_string()];
    let error_scope = vec!["src/error.rs".to_string()];

    cache.last_results.insert(
        (root.clone(), expectation.id.clone(), LastResultStatus::Pass),
        Some(test_last_result(
            LastResultStatus::Pass,
            &pass_scope,
            "1970-01-01T00:00:01Z",
        )),
    );
    cache.last_results.insert(
        (root.clone(), expectation.id.clone(), LastResultStatus::Fail),
        Some(test_last_result(
            LastResultStatus::Fail,
            &fail_scope,
            "1970-01-01T00:00:02Z",
        )),
    );
    cache.last_results.insert(
        (
            root.clone(),
            expectation.id.clone(),
            LastResultStatus::Error,
        ),
        Some(test_last_result(
            LastResultStatus::Error,
            &error_scope,
            "1970-01-01T00:00:03Z",
        )),
    );

    assert_eq!(
        cache
            .read_stored_q_scope(&root, &expectation)
            .unwrap()
            .unwrap(),
        error_scope
    );
    let _ = fs::remove_dir_all(root);
}

fn test_expectation() -> SelectedExpectation {
    SelectedExpectation {
        number: 1,
        id: "abc123".to_string(),
        display_id: "a".to_string(),
        question: "Does it pass?".to_string(),
        expected_answer: "yes".to_string(),
        instructions: String::new(),
        diff_from: crate::config_types::DEFAULT_DIFF_FROM.to_string(),
        target: None,
        question_answer_only: false,
        agent: AgentConfig::default(),
        cooldown: None,
    }
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

fn test_record(
    expectation: &SelectedExpectation,
    scope: &[String],
    observed: &str,
    error: Option<&str>,
) -> CheckRecord {
    CheckRecord {
        timestamp: crate::time::format_record_timestamp(1),
        number: expectation.number,
        result: CheckResult::from_expected_answer(&expectation.expected_answer, observed),
        question: Some(expectation.question.clone()),
        expected_answer: Some(expectation.expected_answer.clone()),
        observed: observed.to_string(),
        error: error.map(str::to_string),
        evidence: "evidence".to_string(),
        scope: scope.to_vec(),
        question_scope_suggestion: Some(scope.to_vec()),
        visible_tree_oid: "visible-tree".to_string(),
        id: expectation.id.clone(),
        display_id: expectation.display_id.clone(),
    }
}

fn test_last_result(
    status: LastResultStatus,
    scope: &[String],
    updated_timestamp: &str,
) -> LastResult {
    let response = match status {
        LastResultStatus::Pass => json!({
            "answer": "yes",
            "evidence": "evidence",
            "qScopeSuggestion": scope,
        }),
        LastResultStatus::Fail => json!({
            "answer": "no",
            "evidence": "evidence",
            "qScopeSuggestion": scope,
        }),
        LastResultStatus::Error => json!({
            "error": "ScopeTooNarrow",
            "evidence": "evidence",
            "qScopeSuggestion": scope,
        }),
    };
    LastResult {
        response_timestamp: "1970-01-01T00:00:01Z".to_string(),
        updated_timestamp: updated_timestamp.to_string(),
        status,
        response,
        q_scope: scope.to_vec(),
        visible_scope: scope.to_vec(),
        checked_tree_oid: (status == LastResultStatus::Pass).then(|| "checked-tree".to_string()),
        visible_tree_oid: matches!(status, LastResultStatus::Pass | LastResultStatus::Fail)
            .then(|| "visible-tree".to_string()),
    }
}

fn read_json(root: &Path, id: &str, file_name: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(last_result_path(root, id, file_name)).unwrap())
        .unwrap()
}

fn last_result_path(root: &Path, id: &str, file_name: &str) -> PathBuf {
    root.join(".git")
        .join("canon")
        .join("xpecs")
        .join(id)
        .join(file_name)
}

fn git_project(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-tmp")
        .join(format!("canon-test-{}-{}-{}", name, process::id(), unique));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let output = Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    root
}
