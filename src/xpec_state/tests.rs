use super::*;
use crate::check::{CheckRecord, CheckResult, SelectedExpectation};
use crate::config_types::AgentConfig;
use crate::git::{TreeSource, VisibleTreeOidCache};
use crate::hash::full_scope;
use serde_json::Value;
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

fn test_expectation() -> SelectedExpectation {
    SelectedExpectation {
        number: 1,
        id: "abc123".to_string(),
        display_id: "a".to_string(),
        question: "Does it pass?".to_string(),
        expected_answer: "yes".to_string(),
        instructions: String::new(),
        target: None,
        question_answer_only: false,
        agent: AgentConfig::default(),
        cooldown: None,
    }
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

fn read_json(root: &Path, id: &str, file_name: &str) -> Value {
    let path = root
        .join(".git")
        .join("canon")
        .join("xpecs")
        .join(id)
        .join(file_name);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
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
