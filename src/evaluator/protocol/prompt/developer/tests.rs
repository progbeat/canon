use super::super::{DeveloperInstructionsContext, EvaluatorPromptMode, PromptRenderer};
use crate::platform::filesystem::create_private_dir;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test] // xpec: Ka
fn developer_instructions_include_transcript_outside_in_place_mode() {
    let rendered = developer_instructions_for_mode(false, false);

    // [Ka] These strings are canon-defined evaluator-facing prompt output,
    // rather than incidental renderer formatting.
    assert!(rendered.contains("Use the transcript below only for context/navigation"));
    assert!(rendered.contains("$ git diff --numstat"));
    assert!(!rendered.contains("$ git diff -- \"$@\""));
    assert!(rendered.contains("$ exec sandbox-sh --read-only --no-git -- \"$@\""));
    assert!(rendered.contains("You are now in the read-only materialized checked project view"));
    assert!(rendered.contains("5 project files are hidden."));
}

#[test] // xpec: Ka,90
fn developer_instructions_omit_git_context_in_in_place_mode() {
    let rendered = developer_instructions_for_mode(true, true);

    assert!(rendered.contains("Custom expectation instructions."));
    // [Ka,90] In-place mode's evaluator-facing prompt contract omits the
    // entire Git-backed transcript branch.
    assert!(!rendered.contains("Use the transcript below only for context/navigation"));
    assert!(!rendered.contains("$ git diff --numstat"));
    assert!(!rendered.contains("$ git diff"));
    assert!(!rendered.contains("$ exec sandbox-sh"));
}

#[test] // xpec: hj,90
fn in_place_prompt_mode_cannot_target_the_diff() {
    assert!(!EvaluatorPromptMode::InPlace.target_is_diff());
}

#[test] // xpec: Ka
fn target_diff_developer_instructions_execute_diff_with_visible_scope() {
    let root = test_output_dir("developer-instructions-scope-repo");
    run_git(&root, &["init", "--quiet"]);
    fs::write(root.join("scoped.txt"), "scoped before\n").unwrap();
    fs::write(root.join("outside.txt"), "outside before\n").unwrap();
    run_git(&root, &["add", "scoped.txt", "outside.txt"]);
    let base_tree_oid = run_git(&root, &["write-tree"]);
    fs::write(root.join("scoped.txt"), "scoped after\n").unwrap();
    fs::write(root.join("outside.txt"), "outside after\n").unwrap();
    run_git(&root, &["add", "scoped.txt", "outside.txt"]);
    let checked_tree_oid = run_git(&root, &["write-tree"]);
    let visible_scope = vec!["scoped.txt".to_string()];

    let rendered =
        PromptRenderer::new(crate::platform::filesystem::PrivateTemporaryDirectoryAllocator::new())
            .developer_instructions(DeveloperInstructionsContext {
                root: &root,
                mode: EvaluatorPromptMode::GitDiff {
                    target_is_diff: true,
                    base_tree_oid: &base_tree_oid,
                    checked_tree_oid: &checked_tree_oid,
                    git_environment: &[],
                },
                question_context: "",
                visible_scope: &visible_scope,
                num_invisible_files: 1,
            })
            .unwrap()
            .text;

    let scoped_diff = rendered
        .split_once("$ git diff -- \"$@\"\n")
        .unwrap()
        .1
        .split_once("\n$ exec")
        .unwrap()
        .0;
    assert!(scoped_diff.contains("scoped.txt"));
    assert!(scoped_diff.contains("scoped after"));
    assert!(!scoped_diff.contains("outside.txt"), "{rendered}");
    assert!(!scoped_diff.contains("outside after"));
    let _ = fs::remove_dir_all(root);
}

#[test] // xpec: Ka
fn full_scope_with_hidden_files_adds_visible_file_guidance() {
    let rendered = developer_instructions_for_mode(false, true);
    let restricted_scope = developer_instructions_for_mode(false, false);

    // [Ka] This conditional line is part of the canon-defined prompt
    // contract, so its presence is stable observable behavior.
    assert!(rendered.starts_with(&(restricted_scope.clone() + "\n")));
    assert_eq!(
        rendered.lines().count(),
        restricted_scope.lines().count() + 1
    );
}

fn developer_instructions_for_mode(in_place: bool, full_scope: bool) -> String {
    let visible_scope = vec![if full_scope { "." } else { "src" }.to_string()];
    PromptRenderer::new(crate::platform::filesystem::PrivateTemporaryDirectoryAllocator::new())
        .developer_instructions(DeveloperInstructionsContext {
            root: Path::new("."),
            mode: if in_place {
                EvaluatorPromptMode::InPlace
            } else {
                EvaluatorPromptMode::GitDiff {
                    target_is_diff: false,
                    base_tree_oid: "HEAD",
                    checked_tree_oid: "HEAD",
                    git_environment: &[],
                }
            },
            question_context: "Custom expectation instructions.",
            visible_scope: &visible_scope,
            num_invisible_files: 5,
        })
        .unwrap()
        .text
}

fn test_output_dir(label: &str) -> PathBuf {
    let random = getrandom::u64().unwrap();
    let path = std::env::temp_dir().join(format!(
        "canon-prompt-template-output-{label}-{}-{random:016x}",
        std::process::id()
    ));
    create_private_dir(&path).unwrap();
    path
}

fn run_git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    // xpec: Ka
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}
