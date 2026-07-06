use crate::check::{CheckRecord, CheckResult, Cooldown, ResolvedExpectation};
use crate::config_types::{AgentConfig, ExpectationTarget};
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::hash::full_scope;
use crate::xpec_state::{
    cached_last_result_for_expectation, check_record_from_cached_result, latest_non_pass_timestamp,
    refresh_reused_same_tree_last_result, CachedLastResultKind, CachedLastResultLookup,
    CachedResultStatus, LastResult, LastResultStatus, XpecStateCache,
};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};

#[test] // xpec: 8m
fn last_result_files_use_status_dependent_fields_and_last_json_follows_error() {
    let root = git_project("last-result-status-fields");
    let expectation = test_expectation();
    let mut cache = XpecStateCache::default();
    let scope = full_scope();

    let mut pass = test_record(&expectation, &scope, "yes", None);
    add_diff_provenance(&mut pass);
    cache
        .write_last_result_for_record(&root, "checked-tree", &expectation, &pass)
        .unwrap();
    let pass_json = read_json(&root, &expectation.id, "last-pass.json");
    assert_eq!(pass_json["status"], "pass");
    assert!(pass_json.get("id").is_none());
    assert!(pass_json.get("displayId").is_none());
    assert_eq!(pass_json["response"]["qScopeSuggestion"], json!(scope));
    assert_eq!(pass_json["checkedTreeOid"], "checked-tree");
    assert_eq!(pass_json["visibleTreeOid"], "visible-tree");
    assert_eq!(pass_json["diffFrom"], ":checkpoint");
    assert_eq!(
        pass_json["diffFromTreeOid"],
        "1234567890abcdef1234567890abcdef12345678"
    );
    assert!(!last_result_path(&root, &expectation.display_id, "last-pass.json").exists());

    let mut fail = test_record(&expectation, &scope, "no", None);
    add_diff_provenance(&mut fail);
    cache
        .write_last_result_for_record(&root, "checked-tree", &expectation, &fail)
        .unwrap();
    let fail_json = read_json(&root, &expectation.id, "last-fail.json");
    assert_eq!(fail_json["status"], "fail");
    assert!(fail_json.get("checkedTreeOid").is_none());
    assert_eq!(fail_json["visibleTreeOid"], "visible-tree");
    assert_eq!(fail_json["diffFrom"], ":checkpoint");
    assert_eq!(
        fail_json["diffFromTreeOid"],
        "1234567890abcdef1234567890abcdef12345678"
    );

    let mut error = test_record(&expectation, &scope, "unparsable", Some("unparsable"));
    add_diff_provenance(&mut error);
    cache
        .write_last_result_for_record(&root, "checked-tree", &expectation, &error)
        .unwrap();
    let error_json = read_json(&root, &expectation.id, "last-error.json");
    assert_eq!(error_json["status"], "error");
    assert!(error_json.get("checkedTreeOid").is_none());
    assert!(error_json.get("visibleTreeOid").is_none());
    assert_eq!(error_json["diffFrom"], ":checkpoint");
    assert_eq!(
        error_json["diffFromTreeOid"],
        "1234567890abcdef1234567890abcdef12345678"
    );

    let last_json = read_json(&root, &expectation.id, "last.json");
    assert_eq!(last_json, error_json);
    assert!(!last_result_path(&root, &expectation.id, "stored-q-scope.json").exists());

    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: DB8
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

#[test] // xpec: 8m
fn last_result_unexpected_answer_uses_fail_status() {
    let root = git_project("last-result-unexpected-answer");
    let expectation = test_expectation();
    let mut cache = XpecStateCache::default();
    let scope = full_scope();

    let invalid_answer = test_record(&expectation, &scope, "not usable", None);
    cache
        .write_last_result_for_record(&root, "checked-tree", &expectation, &invalid_answer)
        .unwrap();

    let fail_json = read_json(&root, &expectation.id, "last-fail.json");
    assert_eq!(fail_json["status"], "fail");
    assert_eq!(fail_json["response"]["answer"], "not usable");
    assert!(fail_json.get("checkedTreeOid").is_none());
    assert_eq!(fail_json["visibleTreeOid"], "visible-tree");
    assert!(!last_result_path(&root, &expectation.id, "last-error.json").exists());
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: YD
fn last_pass_q_scope_ignores_fail_and_error_results() {
    let root = git_project("last-pass-q-scope-only");
    let expectation = test_expectation();
    let mut cache = XpecStateCache::default();
    let pass_scope = vec!["src/pass.rs".to_string()];
    let fail_scope = vec!["src/fail.rs".to_string()];
    let error_scope = vec!["src/error.rs".to_string()];

    write_last_result_fixture(
        &root,
        &expectation,
        test_last_result(LastResultStatus::Fail, &fail_scope, "1970-01-01T00:00:02Z"),
    );
    write_last_result_fixture(
        &root,
        &expectation,
        test_last_result(
            LastResultStatus::Error,
            &error_scope,
            "1970-01-01T00:00:03Z",
        ),
    );

    assert!(cache
        .read_last_pass_q_scope(&root, &expectation)
        .unwrap()
        .is_none());

    write_last_result_fixture(
        &root,
        &expectation,
        test_last_result(LastResultStatus::Pass, &pass_scope, "1970-01-01T00:00:01Z"),
    );

    let mut cache = XpecStateCache::default();
    assert_eq!(
        cache
            .read_last_pass_q_scope(&root, &expectation)
            .unwrap()
            .unwrap(),
        pass_scope
    );
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: 6,8m
fn absent_persistent_history_root_skips_last_result_writes() {
    let expectation = test_expectation();
    let scope = full_scope();
    let pass = test_record(&expectation, &scope, "yes", None);

    let written = XpecStateCache::default()
        .write_interrogation_last_result_for_record_or_absent_history(
            None,
            "checked-tree",
            &expectation,
            &pass,
        )
        .unwrap();

    assert!(written.is_none());
}

#[test] // xpec: 8m
fn diff_provenance_is_required_only_for_git_backed_interrogation_writes() {
    let root = git_project("git-backed-diff-provenance-required");
    let expectation = test_expectation();
    let scope = full_scope();
    let mut pass = test_record(&expectation, &scope, "yes", None);
    remove_diff_provenance(&mut pass);

    XpecStateCache::default()
        .write_last_result_for_record(&root, "checked-tree", &expectation, &pass)
        .unwrap();
    let pass_json = read_json(&root, &expectation.id, "last-pass.json");
    assert!(pass_json.get("diffFrom").is_none());
    assert!(pass_json.get("diffFromTreeOid").is_none());

    let mut cached_result =
        test_last_result(LastResultStatus::Pass, &scope, "1970-01-01T00:00:01Z");
    cached_result.diff_from = None;
    cached_result.diff_from_tree_oid = None;
    XpecStateCache::default()
        .refresh_last_result_for_checked_tree(&root, "checked-tree", &expectation, &cached_result)
        .unwrap();
    let pass_json = read_json(&root, &expectation.id, "last-pass.json");
    assert!(pass_json.get("diffFrom").is_none());
    assert!(pass_json.get("diffFromTreeOid").is_none());

    let written = XpecStateCache::default()
        .write_last_result_for_record_or_absent_history(
            Some(&root),
            "checked-tree",
            &expectation,
            &pass,
        )
        .unwrap();
    assert!(written.is_some());
    let pass_json = read_json(&root, &expectation.id, "last-pass.json");
    assert!(pass_json.get("diffFrom").is_none());
    assert!(pass_json.get("diffFromTreeOid").is_none());

    let error = XpecStateCache::default()
        .write_interrogation_last_result_for_record_or_absent_history(
            Some(&root),
            "checked-tree",
            &expectation,
            &pass,
        )
        .unwrap_err();
    assert!(error.contains("must include diffFrom and diffFromTreeOid"));

    add_diff_provenance(&mut pass);
    let written = XpecStateCache::default()
        .write_interrogation_last_result_for_record_or_absent_history(
            Some(&root),
            "checked-tree",
            &expectation,
            &pass,
        )
        .unwrap();
    assert!(written.is_some());

    let pass_json = read_json(&root, &expectation.id, "last-pass.json");
    assert_eq!(pass_json["diffFrom"], ":checkpoint");
    assert_eq!(
        pass_json["diffFromTreeOid"],
        "1234567890abcdef1234567890abcdef12345678"
    );
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: 8m
fn last_error_preserves_response_suggestion() {
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

#[test] // xpec: 8m,YD
fn last_pass_stores_applied_scope_and_response_suggestion_separately() {
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
    assert_eq!(
        pass_json["response"]["qScopeSuggestion"],
        json!(full_scope())
    );
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: DB8
fn cached_record_preserves_response_question_scope_suggestion() {
    let root = git_project("cached-record-suggestion");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/narrow.rs"), "narrow\n").unwrap();
    fs::write(root.join("src/extra.rs"), "extra\n").unwrap();
    git(&root, &["add", "src/narrow.rs", "src/extra.rs"]);

    let expectation = test_expectation();
    let q_scope = vec!["src/narrow.rs".to_string()];
    let suggestion = vec!["src/extra.rs".to_string()];
    let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
    let mut record = test_record(&expectation, &q_scope, "yes", None);
    record.visible_tree_oid = VisibleTreeOidCache::new()
        .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &q_scope)
        .unwrap();
    record.diff_from_tree_oid = Some(checked_tree_oid.clone());
    record.question_scope_suggestion = Some(suggestion.clone());

    let mut cache = XpecStateCache::default();
    cache
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &record)
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
    .unwrap()
    .unwrap();
    let cached_record = check_record_from_cached_result(&root, &expectation, &hit).unwrap();

    assert_eq!(hit.status, CachedResultStatus::Pass);
    assert_eq!(hit.kind, CachedLastResultKind::SameTree);
    assert_eq!(cached_record.scope, q_scope);
    assert_eq!(cached_record.question_scope_suggestion, Some(suggestion));
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: 8m
fn last_result_response_preserves_question_scope_suggestion() {
    let root = git_project("last-result-no-suggestion");
    let expectation = test_expectation();
    let scope = full_scope();
    let mut record = test_record(&expectation, &scope, "yes", None);
    record.question_scope_suggestion = Some(full_scope());

    XpecStateCache::default()
        .write_last_result_for_record(&root, "checked-tree", &expectation, &record)
        .unwrap();

    let pass_json = read_json(&root, &expectation.id, "last-pass.json");
    assert_eq!(
        pass_json["response"]["qScopeSuggestion"],
        json!(full_scope())
    );
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: DB8
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

#[test] // xpec: DB8
fn same_tree_result_does_not_reconstruct_q_scope_from_current_agent_ignore() {
    let root = git_project("same-tree-no-current-ignore-reconstruction");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a.rs"), "a\n").unwrap();
    fs::write(root.join("src/b.rs"), "b\n").unwrap();
    git(&root, &["add", "src/a.rs", "src/b.rs"]);

    let mut expectation = test_expectation();
    expectation.agent.ignore = vec!["src/b.rs".to_string()];
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

    fs::write(root.join("src/b.rs"), "changed\n").unwrap();
    git(&root, &["add", "src/b.rs"]);
    expectation.agent.ignore.clear();

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

#[test] // xpec: DB8
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

#[test] // xpec: DB8
fn newer_same_tree_fail_wins_over_older_matching_pass() {
    let root = git_project("same-tree-newer-fail");
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

    let mut pass = test_record(&expectation, &pass_scope, "yes", None);
    pass.timestamp = crate::time::format_record_timestamp(1);
    pass.visible_tree_oid = oid_cache
        .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &pass_scope)
        .unwrap();
    cache
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &pass)
        .unwrap();

    let mut fail = test_record(&expectation, &fail_scope, "no", None);
    fail.timestamp = crate::time::format_record_timestamp(2);
    fail.visible_tree_oid = oid_cache
        .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &fail_scope)
        .unwrap();
    cache
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &fail)
        .unwrap();

    assert!(last_result_path(&root, &expectation.id, "last-fail.json").exists());
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

    assert_eq!(hit.status, CachedResultStatus::Fail);
    assert_eq!(hit.kind, CachedLastResultKind::SameTree);
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: DB8
fn same_tree_result_uses_newer_matching_pass() {
    let root = git_project("same-tree-newer-pass");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a.rs"), "a\n").unwrap();
    git(&root, &["add", "src/a.rs"]);

    let mut expectation = test_expectation();
    expectation.target = Some(ExpectationTarget::Diff);
    let mut cache = XpecStateCache::default();
    let scope = full_scope();
    let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
    let visible_tree_oid = VisibleTreeOidCache::new()
        .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &scope)
        .unwrap();

    let mut fail = test_record(&expectation, &scope, "no", None);
    fail.timestamp = crate::time::format_record_timestamp(1);
    fail.visible_tree_oid = visible_tree_oid.clone();
    cache
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &fail)
        .unwrap();

    let mut pass = test_record(&expectation, &scope, "yes", None);
    pass.timestamp = crate::time::format_record_timestamp(2);
    pass.visible_tree_oid = visible_tree_oid;
    cache
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &pass)
        .unwrap();

    let hit = cached_last_result_for_expectation(
        &root,
        &TreeSource::Staged,
        &expectation,
        &mut cache,
        &mut VisibleTreeOidCache::new(),
        CachedLastResultLookup {
            now: 3,
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

#[test] // xpec: DB8,XH
fn explicit_diff_from_uses_newer_matching_pass() {
    let root = git_project("explicit-diff-from-newer-pass");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a.rs"), "a\n").unwrap();
    git(&root, &["add", "src/a.rs"]);

    let mut expectation = test_expectation();
    expectation.target = Some(ExpectationTarget::Diff);
    expectation.diff_from = crate::config_types::AGAINST_TREE_DIFF_FROM.to_string();
    let mut cache = XpecStateCache::default();
    let scope = full_scope();
    let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
    let visible_tree_oid = VisibleTreeOidCache::new()
        .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &scope)
        .unwrap();

    let mut fail = test_record(&expectation, &scope, "no", None);
    fail.timestamp = crate::time::format_record_timestamp(1);
    fail.visible_tree_oid = visible_tree_oid.clone();
    cache
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &fail)
        .unwrap();

    let mut pass = test_record(&expectation, &scope, "yes", None);
    pass.timestamp = crate::time::format_record_timestamp(2);
    pass.visible_tree_oid = visible_tree_oid;
    cache
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &pass)
        .unwrap();

    let hit = cached_last_result_for_expectation(
        &root,
        &TreeSource::Staged,
        &expectation,
        &mut cache,
        &mut VisibleTreeOidCache::new(),
        CachedLastResultLookup {
            now: 3,
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

#[test] // xpec: 8m,DB8
fn new_fail_keeps_last_pass_checkpoint_and_reusable_same_tree_pass() {
    let root = git_project("new-fail-keeps-last-pass");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a.rs"), "a\n").unwrap();
    fs::write(root.join("src/b.rs"), "b\n").unwrap();
    git(&root, &["add", "src/a.rs", "src/b.rs"]);

    let expectation = test_expectation();
    let mut cache = XpecStateCache::default();
    let pass_scope = vec!["src/a.rs".to_string()];
    let fail_scope = vec!["src/b.rs".to_string()];
    let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
    let mut oid_cache = VisibleTreeOidCache::new();

    let mut pass = test_record(&expectation, &pass_scope, "yes", None);
    pass.visible_tree_oid = oid_cache
        .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &pass_scope)
        .unwrap();
    cache
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &pass)
        .unwrap();

    let mut fail = test_record(&expectation, &fail_scope, "no", None);
    fail.visible_tree_oid = oid_cache
        .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &fail_scope)
        .unwrap();
    cache
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &fail)
        .unwrap();

    let last_pass = cache.read_last_pass(&root, &expectation).unwrap().unwrap();
    assert_eq!(
        last_pass.checked_tree_oid.as_deref(),
        Some(checked_tree_oid.as_str())
    );
    assert!(last_result_path(&root, &expectation.id, "last-fail.json").exists());

    fs::write(root.join("src/b.rs"), "changed\n").unwrap();
    git(&root, &["add", "src/b.rs"]);

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
    assert_eq!(
        hit.result.checked_tree_oid.as_deref(),
        Some(checked_tree_oid.as_str())
    );
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: DB8
fn same_tree_result_reuses_replaced_same_status_record() {
    let root = git_project("same-tree-history-pass");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a.rs"), "a\n").unwrap();
    git(&root, &["add", "src/a.rs"]);

    let expectation = test_expectation();
    let scope = full_scope();
    let mut cache = XpecStateCache::default();

    let checked_tree_oid_a = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
    let visible_tree_oid_a = VisibleTreeOidCache::new()
        .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &scope)
        .unwrap();
    let mut pass_a = test_record(&expectation, &scope, "yes", None);
    pass_a.timestamp = crate::time::format_record_timestamp(1);
    pass_a.visible_tree_oid = visible_tree_oid_a.clone();
    cache
        .write_last_result_for_record(&root, &checked_tree_oid_a, &expectation, &pass_a)
        .unwrap();

    let mut pass_a_newer = test_record(&expectation, &scope, "yes", None);
    pass_a_newer.timestamp = crate::time::format_record_timestamp(2);
    pass_a_newer.visible_tree_oid = visible_tree_oid_a;
    cache
        .write_last_result_for_record(&root, &checked_tree_oid_a, &expectation, &pass_a_newer)
        .unwrap();

    fs::write(root.join("src/a.rs"), "b\n").unwrap();
    git(&root, &["add", "src/a.rs"]);
    let checked_tree_oid_b = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
    let visible_tree_oid_b = VisibleTreeOidCache::new()
        .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &scope)
        .unwrap();
    let mut pass_b = test_record(&expectation, &scope, "yes", None);
    pass_b.timestamp = crate::time::format_record_timestamp(3);
    pass_b.visible_tree_oid = visible_tree_oid_b;
    cache
        .write_last_result_for_record(&root, &checked_tree_oid_b, &expectation, &pass_b)
        .unwrap();

    fs::write(root.join("src/a.rs"), "a\n").unwrap();
    git(&root, &["add", "src/a.rs"]);

    let hit = cached_last_result_for_expectation(
        &root,
        &TreeSource::Staged,
        &expectation,
        &mut cache,
        &mut VisibleTreeOidCache::new(),
        CachedLastResultLookup {
            now: 3,
            include_same_tree: true,
            include_cooldown: true,
        },
    )
    .unwrap()
    .unwrap();

    assert_eq!(hit.status, CachedResultStatus::Pass);
    assert_eq!(hit.kind, CachedLastResultKind::SameTree);
    assert_eq!(
        hit.result.response_timestamp,
        crate::time::format_record_timestamp(2)
    );
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: DB8
fn same_tree_history_distinguishes_diff_provenance_for_matching_records() {
    let root = git_project("same-tree-history-diff-provenance");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a.rs"), "a\n").unwrap();
    git(&root, &["add", "src/a.rs"]);

    let expectation = test_expectation();
    let scope = full_scope();
    let mut cache = XpecStateCache::default();
    let checked_tree_oid = TreeSource::Staged.tree_oid_for_prompt_diff(&root).unwrap();
    let visible_tree_oid = VisibleTreeOidCache::new()
        .visible_tree_oid(&root, &TreeSource::Staged, &expectation.agent, &scope)
        .unwrap();

    let mut checkpoint_pass = test_record(&expectation, &scope, "yes", None);
    checkpoint_pass.timestamp = crate::time::format_record_timestamp(1);
    checkpoint_pass.visible_tree_oid = visible_tree_oid.clone();
    checkpoint_pass.diff_from = Some(":checkpoint".to_string());
    checkpoint_pass.diff_from_tree_oid =
        Some("1111111111111111111111111111111111111111".to_string());
    cache
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &checkpoint_pass)
        .unwrap();

    let mut against_pass = test_record(&expectation, &scope, "yes", None);
    against_pass.timestamp = crate::time::format_record_timestamp(2);
    against_pass.visible_tree_oid = visible_tree_oid.clone();
    against_pass.diff_from = Some(":against-tree".to_string());
    against_pass.diff_from_tree_oid = Some("2222222222222222222222222222222222222222".to_string());
    cache
        .write_last_result_for_record(&root, &checked_tree_oid, &expectation, &against_pass)
        .unwrap();

    let mut newer_checkpoint_pass = test_record(&expectation, &scope, "yes", None);
    newer_checkpoint_pass.timestamp = crate::time::format_record_timestamp(3);
    newer_checkpoint_pass.visible_tree_oid = visible_tree_oid;
    newer_checkpoint_pass.diff_from = Some(":checkpoint".to_string());
    newer_checkpoint_pass.diff_from_tree_oid =
        Some("1111111111111111111111111111111111111111".to_string());
    cache
        .write_last_result_for_record(
            &root,
            &checked_tree_oid,
            &expectation,
            &newer_checkpoint_pass,
        )
        .unwrap();

    let retained = cache
        .read_same_tree_records(&root, &expectation, LastResultStatus::Pass)
        .unwrap();
    assert_eq!(retained.len(), 2);
    assert!(retained.iter().any(|result| {
        result.response_timestamp == crate::time::format_record_timestamp(1)
            && result.diff_from.as_deref() == Some(":checkpoint")
    }));
    assert!(retained.iter().any(|result| {
        result.response_timestamp == crate::time::format_record_timestamp(2)
            && result.diff_from.as_deref() == Some(":against-tree")
    }));
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: e5
fn latest_non_pass_timestamp_includes_replaced_fail_records() {
    let root = git_project("latest-non-pass-replaced-fail");
    let expectation = test_expectation();
    let scope = full_scope();
    let mut cache = XpecStateCache::default();

    let mut newer_fail = test_record(&expectation, &scope, "no", None);
    newer_fail.timestamp = crate::time::format_record_timestamp(10);
    cache
        .write_last_result_for_record(&root, "checked-tree", &expectation, &newer_fail)
        .unwrap();

    let mut older_fail = test_record(&expectation, &scope, "no", None);
    older_fail.timestamp = crate::time::format_record_timestamp(5);
    cache
        .write_last_result_for_record(&root, "checked-tree", &expectation, &older_fail)
        .unwrap();

    assert_eq!(
        latest_non_pass_timestamp(&root, &expectation, &mut cache).unwrap(),
        Some(10)
    );
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: DB8
fn git_backed_cooldown_can_reuse_older_pass_when_fail_is_newer() {
    let root = git_project("cooldown-older-pass");
    let mut expectation = test_expectation();
    expectation.cooldown = Some(Cooldown {
        pass_seconds: Some(10),
        fail_seconds: None,
    });
    let mut cache = XpecStateCache::default();
    write_last_result_fixture(
        &root,
        &expectation,
        test_last_result(
            LastResultStatus::Pass,
            &["src/pass.rs".to_string()],
            "1970-01-01T00:00:01Z",
        ),
    );
    write_last_result_fixture(
        &root,
        &expectation,
        test_last_result(
            LastResultStatus::Fail,
            &["src/fail.rs".to_string()],
            "1970-01-01T00:00:02Z",
        ),
    );

    let hit = cached_last_result_for_expectation(
        &root,
        &TreeSource::Staged,
        &expectation,
        &mut cache,
        &mut VisibleTreeOidCache::new(),
        CachedLastResultLookup {
            now: 2,
            include_same_tree: false,
            include_cooldown: true,
        },
    )
    .unwrap()
    .unwrap();

    assert_eq!(hit.status, CachedResultStatus::Pass);
    assert_eq!(hit.kind, CachedLastResultKind::Cooldown);
    assert_eq!(hit.result.status, LastResultStatus::Pass);
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: DB8
fn git_backed_cooldown_can_reuse_older_fail_when_pass_is_newer() {
    let root = git_project("cooldown-older-fail");
    let mut expectation = test_expectation();
    expectation.cooldown = Some(Cooldown {
        pass_seconds: None,
        fail_seconds: Some(10),
    });
    let mut cache = XpecStateCache::default();
    write_last_result_fixture(
        &root,
        &expectation,
        test_last_result(
            LastResultStatus::Fail,
            &["src/fail.rs".to_string()],
            "1970-01-01T00:00:01Z",
        ),
    );
    write_last_result_fixture(
        &root,
        &expectation,
        test_last_result(
            LastResultStatus::Pass,
            &["src/pass.rs".to_string()],
            "1970-01-01T00:00:02Z",
        ),
    );

    let hit = cached_last_result_for_expectation(
        &root,
        &TreeSource::Staged,
        &expectation,
        &mut cache,
        &mut VisibleTreeOidCache::new(),
        CachedLastResultLookup {
            now: 2,
            include_same_tree: false,
            include_cooldown: true,
        },
    )
    .unwrap()
    .unwrap();

    assert_eq!(hit.status, CachedResultStatus::Pass);
    assert_eq!(hit.kind, CachedLastResultKind::Cooldown);
    assert_eq!(hit.result.status, LastResultStatus::Fail);
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: DB8
fn git_backed_cooldown_uses_response_timestamp_when_both_statuses_match() {
    let root = git_project("cooldown-response-timestamp");
    let mut expectation = test_expectation();
    expectation.cooldown = Some(Cooldown {
        pass_seconds: Some(10),
        fail_seconds: Some(10),
    });
    let mut cache = XpecStateCache::default();
    write_last_result_fixture(
        &root,
        &expectation,
        test_last_result_with_response_timestamp(
            LastResultStatus::Pass,
            &["src/pass.rs".to_string()],
            "1970-01-01T00:00:01Z",
            "1970-01-01T00:00:09Z",
        ),
    );
    write_last_result_fixture(
        &root,
        &expectation,
        test_last_result_with_response_timestamp(
            LastResultStatus::Fail,
            &["src/fail.rs".to_string()],
            "1970-01-01T00:00:04Z",
            "1970-01-01T00:00:02Z",
        ),
    );

    let hit = cached_last_result_for_expectation(
        &root,
        &TreeSource::Staged,
        &expectation,
        &mut cache,
        &mut VisibleTreeOidCache::new(),
        CachedLastResultLookup {
            now: 5,
            include_same_tree: false,
            include_cooldown: true,
        },
    )
    .unwrap()
    .unwrap();

    assert_eq!(hit.status, CachedResultStatus::Pass);
    assert_eq!(hit.kind, CachedLastResultKind::Cooldown);
    assert_eq!(hit.result.status, LastResultStatus::Fail);
    let _ = fs::remove_dir_all(root);
}

fn test_expectation() -> ResolvedExpectation {
    ResolvedExpectation {
        number: 1,
        id: "abc123".to_string(),
        display_id: "a".to_string(),
        question: "Does it pass?".to_string(),
        expected_answer: "yes".to_string(),
        question_context: String::new(),
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
    expectation: &ResolvedExpectation,
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
        diff_from: Some(":checkpoint".to_string()),
        diff_from_tree_oid: Some("1234567890abcdef1234567890abcdef12345678".to_string()),
        diff_from_tree_oid_abbrev: Some("1234567".to_string()),
        id: expectation.id.clone(),
        display_id: expectation.display_id.clone(),
    }
}

fn test_last_result(
    status: LastResultStatus,
    scope: &[String],
    updated_timestamp: &str,
) -> LastResult {
    test_last_result_with_response_timestamp(
        status,
        scope,
        "1970-01-01T00:00:01Z",
        updated_timestamp,
    )
}

fn test_last_result_with_response_timestamp(
    status: LastResultStatus,
    scope: &[String],
    response_timestamp: &str,
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
        response_timestamp: response_timestamp.to_string(),
        updated_timestamp: updated_timestamp.to_string(),
        status,
        response,
        q_scope: scope.to_vec(),
        visible_scope: scope.to_vec(),
        checked_tree_oid: (status == LastResultStatus::Pass).then(|| "checked-tree".to_string()),
        visible_tree_oid: matches!(status, LastResultStatus::Pass | LastResultStatus::Fail)
            .then(|| "visible-tree".to_string()),
        diff_from: Some(":checkpoint".to_string()),
        diff_from_tree_oid: Some("1234567890abcdef1234567890abcdef12345678".to_string()),
    }
}

fn add_diff_provenance(record: &mut CheckRecord) {
    record.diff_from = Some(":checkpoint".to_string());
    record.diff_from_tree_oid = Some("1234567890abcdef1234567890abcdef12345678".to_string());
}

fn remove_diff_provenance(record: &mut CheckRecord) {
    record.diff_from = None;
    record.diff_from_tree_oid = None;
}

fn read_json(root: &Path, id: &str, file_name: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(last_result_path(root, id, file_name)).unwrap())
        .unwrap()
}

fn write_last_result_fixture(root: &Path, expectation: &ResolvedExpectation, result: LastResult) {
    let file_name = match result.status {
        LastResultStatus::Pass => "last-pass.json",
        LastResultStatus::Fail => "last-fail.json",
        LastResultStatus::Error => "last-error.json",
    };
    let path = last_result_path(root, &expectation.id, file_name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_string(&result).unwrap()).unwrap();
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
