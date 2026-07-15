use super::runtime::{
    allocate_prompt_template_artifact_dir, allocate_prompt_template_artifact_dir_from_candidates,
    template_command_stdout_artifact_path, trim_rendered_prompt_template_output,
    truncated_template_command_output, ShTranscriptMarkers, PROMPT_TEMPLATE_ARTIFACT_DIR_PREFIX,
};
use super::*;
use crate::platform::create_private_dir;
use crate::xpec_state::LastResultStatus;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

#[test] // xpec: p
fn developer_instructions_include_transcript_outside_in_place_mode() {
    let rendered = developer_instructions_for_mode(false);

    assert!(rendered.contains("Use the transcript below only for context/navigation"));
    assert!(rendered.contains("$ git diff --numstat $BASE_TREE $CHECKED_TREE"));
    assert!(rendered.contains("$ git diff $BASE_TREE $CHECKED_TREE"));
    assert!(rendered.contains("$ enter-sandbox --scope [\"src\"] --ignore []"));
}

#[test] // xpec: p
fn developer_instructions_omit_transcript_in_in_place_mode() {
    let rendered = developer_instructions_for_mode(true);

    assert!(rendered.contains("Custom expectation instructions."));
    assert!(!rendered.contains("Use the transcript below only for context/navigation"));
    assert!(!rendered.contains("$ git diff --numstat"));
    assert!(!rendered.contains("$ git diff"));
    assert!(!rendered.contains("$ enter-sandbox"));
}

#[test] // xpec: 38
fn template_command_output_truncates_large_output() {
    let output = (0..6000)
        .map(|index| format!("line {index}\n"))
        .collect::<String>();
    let output_dir = test_output_dir("truncate");
    let artifact_paths = Mutex::new(Vec::new());
    let rendered = truncated_template_command_output(
        output.as_bytes(),
        &PromptTemplateArtifactDir::Fixed(output_dir.clone()),
        &artifact_paths,
    )
    .unwrap();

    assert!(rendered.contains("[truncated: showing first "));
    assert!(rendered.contains("; full output: "));
    assert!(!rendered.contains("[begin untrusted command output"));
    assert!(!rendered.contains("[end untrusted command output"));
    assert_eq!(artifact_paths.lock().unwrap().len(), 1);
    let _ = fs::remove_dir_all(output_dir);
}

#[test] // xpec: 38,d
fn template_command_output_file_is_content_addressed_and_deduplicated() {
    let output = (0..1200)
        .map(|index| format!("line {index}\n"))
        .collect::<String>();
    let output_dir = test_output_dir("dedupe");
    let artifact_paths = Mutex::new(Vec::new());

    let first = truncated_template_command_output(
        output.as_bytes(),
        &PromptTemplateArtifactDir::Fixed(output_dir.clone()),
        &artifact_paths,
    )
    .unwrap();
    let second = truncated_template_command_output(
        output.as_bytes(),
        &PromptTemplateArtifactDir::Fixed(output_dir.clone()),
        &artifact_paths,
    )
    .unwrap();
    let first_path = PathBuf::from(full_output_path_from_rendered(&first));
    let second_path = PathBuf::from(full_output_path_from_rendered(&second));

    assert_eq!(first_path, second_path);
    assert_eq!(
        artifact_paths.lock().unwrap().as_slice(),
        &[first_path.clone(), first_path.clone()]
    );
    assert!(first_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("canon-template-output-sha256-"));
    assert_eq!(fs::read(&first_path).unwrap(), output.as_bytes());
    let _ = fs::remove_dir_all(output_dir);
}

#[test] // xpec: 38
fn sh_transcript_boundary_whitespace_survives_outer_trim() {
    let output_dir = test_output_dir("sh-boundary-trim");
    let mut artifact_paths = Vec::new();

    let rendered = render_minijinja_resource_template(
        Path::new("."),
        PromptTemplateArtifactDir::Fixed(output_dir.clone()),
        &mut artifact_paths,
        " \n{% filter sh(display=\"printf kept\") %}printf '  kept\\n'{% endfilter %}\n ",
        &[],
        json!({}),
    )
    .unwrap();

    assert_eq!(rendered, "$ printf kept\n  kept\n");
    let _ = fs::remove_dir_all(output_dir);
}

#[test] // xpec: 38,M
fn prompt_template_output_dir_allocations_are_fresh() {
    let first = allocate_prompt_template_artifact_dir().unwrap();
    let second = allocate_prompt_template_artifact_dir().unwrap();

    assert_ne!(first.path(), second.path());
    assert!(first.path().is_dir());
    assert!(second.path().is_dir());
    for path in [first.path(), second.path()] {
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(PROMPT_TEMPLATE_ARTIFACT_DIR_PREFIX));
    }
}

#[test] // xpec: M
fn prompt_template_output_dir_does_not_reuse_fixed_temp_path() {
    let fixed = std::env::temp_dir().join(PROMPT_TEMPLATE_ARTIFACT_DIR_PREFIX);

    let output = allocate_prompt_template_artifact_dir().unwrap();

    assert_ne!(output.path(), fixed);
    assert!(output.path().is_dir());
}

#[test] // xpec: M
fn prompt_template_output_dir_prefers_memory_backed_parent() {
    let memory_backed_parent = test_output_dir("memory-backed-parent");
    let fallback_parent = test_output_dir("fallback-parent");
    let memory_backed_candidates = vec![memory_backed_parent.clone()];
    let fallback_candidates = vec![fallback_parent.clone()];

    let output = allocate_prompt_template_artifact_dir_from_candidates(
        &memory_backed_candidates,
        &fallback_candidates,
    )
    .unwrap();
    let output_path = output.path().to_path_buf();

    assert!(output_path.starts_with(&memory_backed_parent));
    assert!(!output_path.starts_with(&fallback_parent));
    drop(output);
    let _ = fs::remove_dir_all(memory_backed_parent);
    let _ = fs::remove_dir_all(fallback_parent);
}

#[test] // xpec: M
fn prompt_template_output_dir_falls_back_when_memory_backed_parent_is_unavailable() {
    let missing_parent = std::env::temp_dir().join(format!(
        "canon-missing-memory-backed-parent-{}",
        std::process::id()
    ));
    let fallback_parent = test_output_dir("fallback-parent");
    let memory_backed_candidates = vec![missing_parent];
    let fallback_candidates = vec![fallback_parent.clone()];

    let output = allocate_prompt_template_artifact_dir_from_candidates(
        &memory_backed_candidates,
        &fallback_candidates,
    )
    .unwrap();
    let output_path = output.path().to_path_buf();

    assert!(output_path.starts_with(&fallback_parent));
    drop(output);
    let _ = fs::remove_dir_all(fallback_parent);
}

#[test] // xpec: 38,d
fn prompt_template_output_dir_cache_reuses_artifact_dir() {
    let first;
    {
        let cache = PromptTemplateOutputDirCache::new();

        first = cache.path_for_prompt_artifacts().unwrap();
        let second = cache.path_for_prompt_artifacts().unwrap();

        assert_eq!(first, second);
        assert!(first.is_dir());
    }
    assert!(!first.exists());
}

#[test] // xpec: 38
fn prompt_template_output_dir_caches_use_distinct_artifact_dirs() {
    let first;
    let second;
    {
        let first_cache = PromptTemplateOutputDirCache::new();
        let second_cache = PromptTemplateOutputDirCache::new();

        first = first_cache.path_for_prompt_artifacts().unwrap();
        second = second_cache.path_for_prompt_artifacts().unwrap();

        assert_ne!(first, second);
        assert!(first.is_dir());
        assert!(second.is_dir());
    }
    assert!(!first.exists());
    assert!(!second.exists());
}

#[test] // xpec: 38
fn template_command_stdout_path_is_deterministic_within_run_output_dir() {
    let output_dir = test_output_dir("same-run-content-addressed");
    let stdout = b"same complete stdout";

    let first = template_command_stdout_artifact_path(&output_dir, stdout);
    let second = template_command_stdout_artifact_path(&output_dir, stdout);

    assert_eq!(first, second);
    assert_eq!(first.parent(), Some(output_dir.as_path()));
    assert!(first
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("canon-template-output-sha256-"));
    let _ = fs::remove_dir_all(output_dir);
}

#[test] // xpec: 38
fn template_command_output_file_preserves_raw_stdout_bytes() {
    // The saved file is raw command stdout. The full rendered prompt string
    // is trimmed separately after all `sh` filters return.
    let mut output = (0..1200)
        .flat_map(|index| format!("line {index}\n").into_bytes())
        .collect::<Vec<_>>();
    output.extend_from_slice(&[0xff, 0xfe, b'\n']);
    let output_dir = test_output_dir("raw-bytes");
    let artifact_paths = Mutex::new(Vec::new());

    let rendered = truncated_template_command_output(
        &output,
        &PromptTemplateArtifactDir::Fixed(output_dir.clone()),
        &artifact_paths,
    )
    .unwrap();
    let path = full_output_path_from_rendered(&rendered);
    let saved = fs::read(path).unwrap();

    assert_eq!(saved, output);
    assert_eq!(
        artifact_paths.lock().unwrap().as_slice(),
        &[PathBuf::from(path)]
    );
    let saved_path = Path::new(path);
    assert_eq!(saved_path.parent(), Some(output_dir.as_path()));
    let _ = fs::remove_dir_all(output_dir);
}

fn full_output_path_from_rendered(rendered: &str) -> &str {
    rendered
        .lines()
        .find(|line| line.starts_with("[truncated: "))
        .unwrap()
        .strip_suffix(']')
        .unwrap()
        .rsplit_once("full output: ")
        .unwrap()
        .1
}

fn developer_instructions_for_mode(in_place: bool) -> String {
    let output_dir = test_output_dir(if in_place {
        "developer-instructions-in-place"
    } else {
        "developer-instructions-normal"
    });
    let mut artifact_paths = Vec::new();
    let visible_scope = vec!["src".to_string()];
    let ignore = Vec::new();
    let rendered = developer_instructions(DeveloperInstructionsContext {
        root: Path::new("."),
        template_artifact_dir: PromptTemplateArtifactDir::Fixed(output_dir.clone()),
        template_artifact_paths: &mut artifact_paths,
        in_place,
        diff_from_tree_oid: "HEAD",
        checked_tree_oid: "HEAD",
        question_context: "Custom expectation instructions.",
        q_scope: &visible_scope,
        ignore: &ignore,
        visible_scope: &visible_scope,
        checked_file_count: 10,
        visible_file_count: 5,
        last_pass: None,
    })
    .unwrap();
    let _ = fs::remove_dir_all(output_dir);
    rendered
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

#[test] // xpec: 2
fn target_diff_prompt_hint_uses_full_q_scope_suggestion() {
    let last_pass = LastResult {
        response_timestamp: "1970-01-01T00:00:01Z".to_string(),
        updated_timestamp: "1970-01-01T00:00:01Z".to_string(),
        status: LastResultStatus::Pass,
        response: json!({
            "answer": "yes",
            "evidence": "`src/a.rs`",
            "qScopeSuggestion": ["src/a.rs"],
        }),
        q_scope: vec!["src/a.rs".to_string()],
        visible_scope: vec!["src/a.rs".to_string()],
        checked_tree_oid: Some("checked-tree".to_string()),
        visible_tree_oid: Some("visible-tree".to_string()),
        diff_from: None,
        diff_from_tree_oid: None,
    };
    let output_dir = test_output_dir("turn-prompt");
    let mut artifact_paths = Vec::new();

    let prompt = evaluator_turn_prompt(EvaluatorTurnPromptContext {
        root: Path::new("."),
        template_artifact_dir: PromptTemplateArtifactDir::Fixed(output_dir.clone()),
        template_artifact_paths: &mut artifact_paths,
        short_id: "e",
        question: "Does it pass?",
        expected_answer: "yes",
        in_place: false,
        diff_from: crate::config_types::DEFAULT_DIFF_FROM,
        target: Some("diff"),
        last_pass: Some(&last_pass),
    })
    .unwrap();

    assert!(prompt.contains("This question targets the Git diff."));
    assert!(prompt.contains("Use this prior evaluation if it still holds:"));
    assert!(prompt.contains(r#"{"e":"Does it pass?"}"#));
    assert!(prompt.contains(r#""answer":"yes""#));
    assert!(prompt.contains(r#""evidence":"`src/a.rs`""#));
    // The turn prompt provides this response literal to the evaluator. The
    // base instruction to keep a provided response's qScopeSuggestion
    // refers to this rendered literal, not the stored last-pass response
    // that was used as template input.
    assert!(prompt.contains(r#""qScopeSuggestion":["."]"#));
    assert!(!prompt.contains(r#""qScopeSuggestion":["src/a.rs"]"#));
    let _ = fs::remove_dir_all(output_dir);
}

#[test] // xpec: 2
fn target_diff_prompt_uses_expected_answer_when_diff_from_is_not_checkpoint() {
    let last_pass = LastResult {
        response_timestamp: "1970-01-01T00:00:01Z".to_string(),
        updated_timestamp: "1970-01-01T00:00:01Z".to_string(),
        status: LastResultStatus::Pass,
        response: json!({
            "answer": "no",
            "evidence": "`src/a.rs`",
            "qScopeSuggestion": ["src/a.rs"],
        }),
        q_scope: vec!["src/a.rs".to_string()],
        visible_scope: vec!["src/a.rs".to_string()],
        checked_tree_oid: Some("checked-tree".to_string()),
        visible_tree_oid: Some("visible-tree".to_string()),
        diff_from: None,
        diff_from_tree_oid: None,
    };
    let output_dir = test_output_dir("turn-prompt-against-tree");
    let mut artifact_paths = Vec::new();

    let prompt = evaluator_turn_prompt(EvaluatorTurnPromptContext {
        root: Path::new("."),
        template_artifact_dir: PromptTemplateArtifactDir::Fixed(output_dir.clone()),
        template_artifact_paths: &mut artifact_paths,
        short_id: "e",
        question: "Does it pass?",
        expected_answer: "yes",
        in_place: false,
        diff_from: crate::config_types::AGAINST_TREE_DIFF_FROM,
        target: Some("diff"),
        last_pass: Some(&last_pass),
    })
    .unwrap();

    assert!(prompt.contains("This question targets the Git diff."));
    assert!(prompt.contains(r#""evidence":"""#));
    assert!(prompt.contains(r#""answer":"yes""#));
    assert!(!prompt.contains(r#""answer":"no""#));
    let _ = fs::remove_dir_all(output_dir);
}

#[test] // xpec: 2
fn in_place_turn_prompt_omits_target_diff_hint() {
    let last_pass = LastResult {
        response_timestamp: "1970-01-01T00:00:01Z".to_string(),
        updated_timestamp: "1970-01-01T00:00:01Z".to_string(),
        status: LastResultStatus::Pass,
        response: json!({
            "answer": "yes",
            "evidence": "`src/a.rs`",
            "qScopeSuggestion": ["src/a.rs"],
        }),
        q_scope: vec!["src/a.rs".to_string()],
        visible_scope: vec!["src/a.rs".to_string()],
        checked_tree_oid: Some("checked-tree".to_string()),
        visible_tree_oid: Some("visible-tree".to_string()),
        diff_from: None,
        diff_from_tree_oid: None,
    };
    let output_dir = test_output_dir("turn-prompt-in-place");
    let mut artifact_paths = Vec::new();

    let prompt = evaluator_turn_prompt(EvaluatorTurnPromptContext {
        root: Path::new("."),
        template_artifact_dir: PromptTemplateArtifactDir::Fixed(output_dir.clone()),
        template_artifact_paths: &mut artifact_paths,
        short_id: "e",
        question: "Does it pass?",
        expected_answer: "yes",
        in_place: true,
        diff_from: crate::config_types::DEFAULT_DIFF_FROM,
        target: Some("diff"),
        last_pass: Some(&last_pass),
    })
    .unwrap();

    assert_eq!(prompt, r#"{"e":"Does it pass?"}"#);
    let _ = fs::remove_dir_all(output_dir);
}

#[test] // xpec: 38
fn resource_template_rendering_trims_outer_whitespace() {
    let output_dir = test_output_dir("outer-trim");
    let mut artifact_paths = Vec::new();

    let rendered = render_minijinja_resource_template(
        Path::new("."),
        PromptTemplateArtifactDir::Fixed(output_dir.clone()),
        &mut artifact_paths,
        "\n  {{ value }}  \n",
        &[],
        json!({ "value": "answer" }),
    )
    .unwrap();

    assert_eq!(rendered, "answer");
    let _ = fs::remove_dir_all(output_dir);
}

#[test] // xpec: 38
fn outer_trim_preserves_shell_transcript_edges() {
    let markers = test_markers();
    let transcript = "$ cmd\n  output  \n";
    let rendered = format!("\n  {}  \n", markers.wrap_transcript(transcript));

    let trimmed = trim_rendered_prompt_template_output(&rendered, &markers);

    assert_eq!(trimmed, transcript);
}

#[test] // xpec: 38
fn shell_transcript_markers_are_preserved_inside_transcript_text() {
    let markers = test_markers();
    let transcript = format!(
        "$ cmd\n{}{}{}\n",
        markers.start, markers.end, markers.escape
    );
    let rendered = markers.wrap_transcript(&transcript);

    let trimmed = trim_rendered_prompt_template_output(&rendered, &markers);

    assert_eq!(trimmed, transcript);
}

fn test_markers() -> ShTranscriptMarkers {
    ShTranscriptMarkers {
        start: "<start>".to_string(),
        end: "<end>".to_string(),
        escape: "<escape>".to_string(),
    }
}
