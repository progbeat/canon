mod runtime;

use runtime::{
    json_filter, render_with_repository_cwd, shell_args_filter, shell_quote_filter,
    shell_transcript_filter, trim_rendered_prompt_template_output, ShTranscriptMarkers,
};
pub(crate) use runtime::{PromptTemplateArtifactDir, PromptTemplateOutputDirCache};

use crate::xpec_state::LastResult;
use minijinja::value::Kwargs;
use minijinja::{Environment, Error};
use serde_json::{json, Value as JsonValue};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// Canon-owned evaluator templates are loaded from `resources/prompts/`; this
// module only renders those resource files with runtime check data.
const DEVELOPER_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("../../../resources/prompts/evaluator_developer_instructions.txt");
const EVALUATOR_TURN_PROMPT_TEMPLATE: &str =
    include_str!("../../../resources/prompts/evaluator_turn_prompt.txt");

pub(crate) struct DeveloperInstructionsContext<'a> {
    pub(crate) root: &'a Path,
    pub(crate) template_artifact_dir: PromptTemplateArtifactDir,
    pub(crate) template_artifact_paths: &'a mut Vec<PathBuf>,
    pub(crate) in_place: bool,
    pub(crate) diff_from_tree_oid: &'a str,
    pub(crate) checked_tree_oid: &'a str,
    // Data for the resource template's `xpec.instructions` variable.
    pub(crate) question_context: &'a str,
    pub(crate) q_scope: &'a [String],
    pub(crate) ignore: &'a [String],
    pub(crate) visible_scope: &'a [String],
    pub(crate) checked_file_count: usize,
    pub(crate) visible_file_count: usize,
    pub(crate) last_pass: Option<&'a LastResult>,
}

pub(crate) fn developer_instructions(
    context: DeveloperInstructionsContext<'_>,
) -> Result<String, String> {
    // This count is reporting-only prompt data. File visibility has already
    // been decided by the visible-tree pathspec selection in
    // `src/git/visible_tree_oid/`; the template's "likely unnecessary" wording
    // does not add another hiding rule.
    let files_not_selected_by_visible_scope_pathspec = context
        .checked_file_count
        .checked_sub(context.visible_file_count)
        .ok_or("visible file count exceeds checked file count")?;
    // The transcript intentionally has two scoped diff views over
    // `visible_scope`: `git diff --numstat` for change discovery, then detailed
    // `git diff` for inspectable content. Template display text omits the
    // pathspec so developer instructions show the relevant tree OIDs without
    // repeating noisy scope arguments.
    render_minijinja_resource_template(
        context.root,
        context.template_artifact_dir,
        context.template_artifact_paths,
        DEVELOPER_INSTRUCTIONS_TEMPLATE,
        &[
            ("BASE_TREE", context.diff_from_tree_oid),
            ("CHECKED_TREE", context.checked_tree_oid),
        ],
        json!({
            "xpec": {
                // [UZ] This is human-authored expectation context rendered by
                // the resource template, not another implementation-owned
                // evaluator prompt or instruction source.
                "instructions": context.question_context,
                "q_scope": context.q_scope,
                "ignore": context.ignore,
                "visible_scope": context.visible_scope,
            },
            "in_place": context.in_place,
            "last_pass": context.last_pass,
            "num_invisible_files": files_not_selected_by_visible_scope_pathspec,
        }),
    )
}

pub(crate) struct EvaluatorTurnPromptContext<'a> {
    pub(crate) root: &'a Path,
    pub(crate) template_artifact_dir: PromptTemplateArtifactDir,
    pub(crate) template_artifact_paths: &'a mut Vec<PathBuf>,
    pub(crate) short_id: &'a str,
    pub(crate) question: &'a str,
    pub(crate) expected_answer: &'a str,
    pub(crate) in_place: bool,
    pub(crate) diff_from: &'a str,
    pub(crate) target: Option<&'a str>,
    pub(crate) last_pass: Option<&'a LastResult>,
}

pub(crate) fn evaluator_turn_prompt(
    context: EvaluatorTurnPromptContext<'_>,
) -> Result<String, String> {
    let (diff_from, target, last_pass) = if context.in_place {
        // In-place mode has no Git diff target or checkpoint context. The
        // caller validates resolved expectations before interrogation; this
        // clamp keeps the rendered prompt diff-free even if an invalid in-place
        // expectation reaches this component.
        ("", None, None)
    } else {
        (context.diff_from, context.target, context.last_pass)
    };
    // `diff_from` is template input for this fresh evaluator turn only. Cached
    // results are emitted without rendering this prompt. The turn template uses
    // `xpec.diff_from` to choose whether a target-diff prompt can reuse the
    // checkpoint response or must render the xpec's default answer.
    // `target` is the same kind of per-turn prompt input; it is deliberately
    // not part of evaluator thread reuse.
    let xpec_context = turn_prompt_xpec_context(
        context.short_id,
        context.question,
        context.expected_answer,
        diff_from,
        target,
    );
    render_minijinja_resource_template(
        context.root,
        context.template_artifact_dir,
        context.template_artifact_paths,
        EVALUATOR_TURN_PROMPT_TEMPLATE,
        &[],
        json!({
            "xpec": xpec_context,
            "last_pass": last_pass,
        }),
    )
}

fn turn_prompt_xpec_context(
    short_id: &str,
    question: &str,
    expected_answer: &str,
    diff_from: &str,
    target: Option<&str>,
) -> JsonValue {
    json!({
        "short_id": short_id,
        "q": question,
        "a": expected_answer,
        "diff_from": diff_from,
        "target": target.unwrap_or(""),
    })
}

fn render_minijinja_resource_template(
    root: &Path,
    template_artifact_dir: PromptTemplateArtifactDir,
    template_artifact_paths: &mut Vec<PathBuf>,
    template: &str,
    template_shell_env: &[(&str, &str)],
    context: JsonValue,
) -> Result<String, String> {
    let mut environment = Environment::new();
    environment.add_filter("json", json_filter);
    environment.add_filter("shq", shell_quote_filter);
    environment.add_filter("shargs", shell_args_filter);
    let command_root = root.to_path_buf();
    let command_env = template_shell_env
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect::<Vec<_>>();
    let command_artifact_paths = Arc::new(Mutex::new(Vec::new()));
    let sh_transcript_markers = ShTranscriptMarkers::new()?;
    let filter_transcript_markers = sh_transcript_markers.clone();
    let filter_artifact_paths = Arc::clone(&command_artifact_paths);
    environment.add_filter(
        "sh",
        move |command: String, kwargs: Kwargs| -> Result<String, Error> {
            let transcript = shell_transcript_filter(
                &command_root,
                &template_artifact_dir,
                filter_artifact_paths.as_ref(),
                &command_env,
                command,
                kwargs,
            )?;
            Ok(filter_transcript_markers.wrap_transcript(&transcript))
        },
    );
    let template = environment
        .template_from_str(template)
        .map_err(|err| format!("failed to parse prompt template: {}", err))?;
    // Prompt Templates require the MiniJinja render itself to start from this
    // check root cwd: the repository root outside in-place mode, or the
    // checked directory in in-place mode.
    let rendered = render_with_repository_cwd(root, || template.render(context))
        .map_err(|err| format!("failed to render prompt template: {}", err))?;
    template_artifact_paths.extend(command_artifact_paths.lock().unwrap().iter().cloned());
    // This is the final rendered prompt trim required by Prompt Templates. It
    // is separate from `sh` command-body trimming in `shell_transcript_filter`.
    // Internal sentinels protect `sh` transcript text at prompt boundaries so
    // command display text, stdout text, saved stdout bytes, and the
    // truncation marker keep their specified spelling.
    Ok(trim_rendered_prompt_template_output(
        &rendered,
        &sh_transcript_markers,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::create_private_dir;
    use crate::xpec_state::LastResultStatus;
    use serde_json::json;
    use std::fs;

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
}
