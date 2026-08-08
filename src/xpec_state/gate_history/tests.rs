use crate::check::{
    CheckRecord, CheckResult, ExpectationIdentity, ResolvedExpectation, ResolvedExpectationKind,
};
use crate::config_types::{AgentConfig, DEFAULT_DIFF_FROM};
use crate::xpec_state::{LastResult, LastResultResponse, LastResultStatus, XpecStateCache};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};

#[test] // xpec: KD,90,Sh
fn canonical_git_history_migrates_to_gate_results() -> Result<(), String> {
    let root = git_project()?;
    let expectation = expectation();
    let mut state = XpecStateCache::default();
    let xpec_dir = state.xpec_dir(&root, &expectation).unwrap();
    fs::create_dir_all(&xpec_dir).unwrap();
    write_canonical_result(
        &xpec_dir,
        &git_backed_result(LastResultStatus::Pass, "pass-tree"),
    );
    write_canonical_result(
        &xpec_dir,
        &git_backed_result(LastResultStatus::Fail, "fail-tree"),
    );

    state
        .retain_only_current_configuration(
            &root,
            &[ExpectationIdentity {
                id: expectation.require_configured_id().unwrap().to_string(),
                display_id: expectation.display_id.clone(),
            }],
        )
        .unwrap();
    state
        .write_last_result_for_record(&root, None, &expectation, &in_place_record())
        .unwrap();
    let cache = state
        .read_gate_results(&root, &expectation)
        .unwrap()
        .unwrap();

    assert_eq!(
        cache
            .last_pass
            .as_ref()
            .map(|pass| pass.visible_tree_oid.as_str()),
        Some("visible-tree")
    );
    assert_eq!(
        cache
            .last_fail
            .as_ref()
            .map(|fail| fail.checked_tree_oid.as_str()),
        Some("fail-tree")
    );
    fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}

fn git_project() -> Result<PathBuf, String> {
    let root = temporary_project_root()?;
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let output = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&root)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(root)
}

fn in_place_record() -> CheckRecord {
    CheckRecord {
        timestamp: "2026-01-02T00:00:00Z".to_string(),
        result: CheckResult::Fail,
        to: crate::config_types::ExpectationTo::Agent,
        question: Some("Does legacy history migrate?".to_string()),
        expected_answer: Some("yes".to_string()),
        observed: "no".to_string(),
        error: None,
        evidence: Some("in-place evidence".to_string()),
        scope: vec![".".to_string()],
        q_scope_suggestion: None,
        visible_tree_oid: None,
        diff_from: None,
        diff_from_tree_oid: None,
        diff_from_tree_oid_abbrev: None,
        id: "legacy-xpec".to_string(),
        display_id: "legacy-xpec".to_string(),
    }
}

fn git_backed_result(status: LastResultStatus, checked_tree_oid: &str) -> LastResult {
    LastResult {
        response_timestamp: "2026-01-01T00:00:00Z".to_string(),
        updated_timestamp: "2026-01-01T00:00:01Z".to_string(),
        status,
        response: LastResultResponse::answered(
            match status {
                LastResultStatus::Pass => "yes",
                LastResultStatus::Fail => "no",
            },
            "test evidence",
            None,
        ),
        q_scope: vec![".".to_string()],
        visible_scope: vec![".".to_string()],
        checked_tree_oid: Some(checked_tree_oid.to_string()),
        visible_tree_oid: (status == LastResultStatus::Pass).then(|| "visible-tree".to_string()),
        diff_from: None,
        diff_from_tree_oid: None,
    }
}

fn write_canonical_result(xpec_dir: &Path, result: &LastResult) {
    let path = xpec_dir.join(result.status.file_name());
    let mut bytes = serde_json::to_vec(result).unwrap();
    bytes.push(b'\n');
    fs::write(path, bytes).unwrap();
}

fn expectation() -> ResolvedExpectation {
    ResolvedExpectation {
        kind: ResolvedExpectationKind::Configured {
            id: "legacy-xpec".to_string(),
        },
        display_id: "legacy-xpec".to_string(),
        to: crate::config_types::ExpectationTo::Agent,
        rank: 0,
        question: "Does legacy history migrate?".to_string(),
        expected_answer: "yes".to_string(),
        question_context: String::new(),
        diff_from: DEFAULT_DIFF_FROM.to_string(),
        target: None,
        agent: AgentConfig::default(),
        cooldown: None,
        q_scope: Default::default(),
    }
}

fn temporary_project_root() -> Result<PathBuf, String> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    Ok(std::env::temp_dir().join(format!(
        "canon-git-backed-result-migration-{}-{unique}",
        process::id()
    )))
}
