use super::*;
use crate::check::interrogation::state::CheckTreeContext;
use crate::config_types::{AgentConfig, CheckConfig, AGAINST_TREE_DIFF_FROM};
use crate::git::TreeSource;
use crate::hash::full_scope;
use crate::materialization::TreeMaterializer;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test] // xpec: Sh,u
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
    let tree_materializer = TreeMaterializer::apply_for_tree_source(&root, source.clone()).unwrap();
    let mut visible_tree_oid_cache = VisibleTreeOidCache::new();
    let tree_context = CheckTreeContext {
        checked_tree_oid: source.tree_oid_for_prompt_diff(&root).unwrap(),
        against_tree_oid: source.tree_oid_for_prompt_diff(&root).unwrap(),
        head_tree_oid: None,
        explicit_diff_from_tree_oids: Default::default(),
        checked_file_count: visible_tree_oid_cache
            .checked_file_count(&root, &source)
            .unwrap(),
        prompt_git_environment: Vec::new(),
    };
    let against_tree_oid = tree_context.against_tree_oid.clone();
    let runtime = CheckRuntime::materialized(
        &root,
        &tree_materializer,
        &source,
        tree_context,
        &config,
        false,
    );
    let expectation = test_expectation(&agent);
    let scope = full_scope();
    let call = InterrogationCall {
        runtime: &runtime,
        expectation: &expectation,
        scope: &scope,
        turn_kind: InterrogationTurnKind::Initial,
        progress: None,
    };
    let mut xpec_state = XpecStateCache::default();
    let result = <InterrogationResult as RecoverableInterrogation>::from_interrogation_error(
        &call,
        EvaluatorError::failure(
            crate::evaluator::EvaluatorFailureKind::Interrupted,
            "typed interruption detail",
        ),
        &mut xpec_state,
        &mut visible_tree_oid_cache,
    )
    .unwrap();
    assert!(result.interrupted);
    let record = result.output;

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
    // xpec: Sh
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn test_expectation(agent: &AgentConfig) -> ResolvedExpectation {
    ResolvedExpectation {
        kind: crate::check::core::ResolvedExpectationKind::Configured {
            id: "expectation-id".to_string(),
        },
        display_id: "e".to_string(),
        to: crate::config_types::ExpectationTo::Agent,
        rank: 0,
        question: "Does this fail technically?".to_string(),
        expected_answer: "yes".to_string(),
        question_context: String::new(),
        diff_from: AGAINST_TREE_DIFF_FROM.to_string(),
        target: None,
        agent: agent.clone(),
        cooldown: None,
        q_scope: Default::default(),
    }
}
