mod runtime;

use runtime::{
    json_filter, render_with_repository_cwd, shell_args_filter, shell_quote_filter,
    shell_transcript_filter, trim_rendered_prompt_template_output, PromptTemplateArtifactDir,
    PromptTemplateOutputDirCache, ShTranscriptMarkers,
};

use crate::xpec_state::LastResult;
use minijinja::value::Kwargs;
use minijinja::{Environment, Error};
use serde_json::{json, Value as JsonValue};
use std::ffi::OsString;
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
    pub(crate) mode: DeveloperInstructionsMode<'a>,
    // Data for the resource template's `xpec.instructions` variable.
    pub(crate) question_context: &'a str,
    pub(crate) visible_scope: &'a [String],
    pub(crate) num_invisible_files: usize,
    pub(crate) last_pass: Option<&'a LastResult>,
}

#[derive(Clone, Copy)]
pub(crate) enum DeveloperInstructionsMode<'a> {
    InPlace,
    GitDiff {
        base_tree_oid: &'a str,
        checked_tree_oid: &'a str,
        git_environment: &'a [(OsString, OsString)],
    },
}

pub(crate) struct EvaluatorTurnPromptContext<'a> {
    pub(crate) root: &'a Path,
    pub(crate) short_id: &'a str,
    pub(crate) question: &'a str,
    pub(crate) expected_answer: &'a str,
    pub(crate) mode: EvaluatorTurnPromptMode<'a>,
}

#[derive(Clone, Copy)]
pub(crate) enum EvaluatorTurnPromptMode<'a> {
    InPlace,
    GitBacked {
        diff_from: &'a str,
        last_pass: Option<&'a LastResult>,
        // [eS] This derived flag is turn-prompt input, not evaluation policy.
        render_target_diff_hint: bool,
    },
}

pub(crate) struct RenderedPrompt {
    pub(crate) text: String,
}

pub(crate) struct PromptRenderer {
    output_dir_cache: Arc<PromptTemplateOutputDirCache>,
}

impl PromptRenderer {
    pub(crate) fn new() -> PromptRenderer {
        PromptRenderer {
            output_dir_cache: Arc::new(PromptTemplateOutputDirCache::new()),
        }
    }

    pub(crate) fn artifact_directory(&self) -> Result<PathBuf, String> {
        self.output_dir_cache.path_for_prompt_artifacts()
    }

    pub(crate) fn developer_instructions(
        &self,
        context: DeveloperInstructionsContext<'_>,
    ) -> Result<RenderedPrompt, String> {
        let mut artifact_paths = Vec::new();
        let text = render_developer_instructions(
            PromptTemplateArtifactDir::Lazy(Arc::clone(&self.output_dir_cache)),
            &mut artifact_paths,
            context,
        )?;
        Ok(RenderedPrompt { text })
    }

    pub(crate) fn evaluator_turn_prompt(
        &self,
        context: EvaluatorTurnPromptContext<'_>,
    ) -> Result<RenderedPrompt, String> {
        let mut artifact_paths = Vec::new();
        let text = render_evaluator_turn_prompt(
            PromptTemplateArtifactDir::Lazy(Arc::clone(&self.output_dir_cache)),
            &mut artifact_paths,
            context,
        )?;
        Ok(RenderedPrompt { text })
    }
}

fn render_developer_instructions(
    template_artifact_dir: PromptTemplateArtifactDir,
    template_artifact_paths: &mut Vec<PathBuf>,
    context: DeveloperInstructionsContext<'_>,
) -> Result<String, String> {
    // xpec: 8O
    // Each `sh` block gets the canonical visible scope as positional shell
    // arguments. The resource can therefore execute its specified `"$@"`
    // commands even though template filters run independently.
    let (in_place, git_diff_environment) = match context.mode {
        DeveloperInstructionsMode::InPlace => (true, Vec::new()),
        DeveloperInstructionsMode::GitDiff {
            base_tree_oid,
            checked_tree_oid,
            git_environment,
        } => (
            false,
            [
                (OsString::from("BASE_TREE"), OsString::from(base_tree_oid)),
                (
                    OsString::from("CHECKED_TREE"),
                    OsString::from(checked_tree_oid),
                ),
            ]
            .into_iter()
            .chain(git_environment.iter().cloned())
            .collect(),
        ),
    };
    render_minijinja_resource_template(
        context.root,
        template_artifact_dir,
        template_artifact_paths,
        DEVELOPER_INSTRUCTIONS_TEMPLATE,
        &git_diff_environment,
        context.visible_scope,
        json!({
            "xpec": {
                // [UZ] This is human-authored expectation context rendered by
                // the resource template, not another implementation-owned
                // evaluator prompt or instruction source.
                "instructions": context.question_context,
                "visible_scope": context.visible_scope,
            },
            "in_place": in_place,
            "last_pass": context.last_pass,
            "num_invisible_files": context.num_invisible_files,
        }),
    )
}

fn render_evaluator_turn_prompt(
    template_artifact_dir: PromptTemplateArtifactDir,
    template_artifact_paths: &mut Vec<PathBuf>,
    context: EvaluatorTurnPromptContext<'_>,
) -> Result<String, String> {
    let (diff_from, target, last_pass) = match context.mode {
        EvaluatorTurnPromptMode::InPlace => ("", None, None),
        EvaluatorTurnPromptMode::GitBacked {
            diff_from,
            render_target_diff_hint,
            last_pass,
        } => (
            diff_from,
            render_target_diff_hint.then_some("diff"),
            last_pass,
        ),
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
        template_artifact_dir,
        template_artifact_paths,
        EVALUATOR_TURN_PROMPT_TEMPLATE,
        &[],
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
    template_shell_env: &[(OsString, OsString)],
    template_shell_args: &[String],
    context: JsonValue,
) -> Result<String, String> {
    let mut environment = Environment::new();
    environment.add_filter("json", json_filter);
    environment.add_filter("shq", shell_quote_filter);
    environment.add_filter("shargs", shell_args_filter);
    let command_root = root.to_path_buf();
    let command_env = template_shell_env.to_vec();
    let command_args = template_shell_args.to_vec();
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
                &command_args,
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
    use crate::xpec_state::{LastResultResponse, LastResultStatus};
    use serde_json::json;
    use std::fs;
    use std::process::Command;

    #[test] // xpec: 8O
    fn developer_instructions_include_transcript_outside_in_place_mode() {
        let rendered = developer_instructions_for_mode(false);

        assert!(rendered.contains("Use the transcript below only for context/navigation"));
        assert!(rendered.contains("$ git diff --shortstat \"$BASE_TREE\" \"$CHECKED_TREE\""));
        assert!(rendered.contains("$ set -- 'src'"));
        assert!(
            rendered.contains("$ git diff --numstat \"$BASE_TREE\" \"$CHECKED_TREE\" -- \"$@\"")
        );
        assert!(rendered.contains("$ git diff \"$BASE_TREE\" \"$CHECKED_TREE\" -- \"$@\""));
        assert!(rendered.contains("$ exec sandbox-sh --read-only --no-git -- \"$@\""));
        assert!(rendered.contains("5 project files are hidden."));
    }

    #[test] // xpec: 8O
    fn developer_instructions_omit_transcript_in_in_place_mode() {
        let rendered = developer_instructions_for_mode(true);

        assert!(rendered.contains("Custom expectation instructions."));
        assert!(!rendered.contains("Use the transcript below only for context/navigation"));
        assert!(!rendered.contains("$ git diff --numstat"));
        assert!(!rendered.contains("$ git diff"));
        assert!(!rendered.contains("$ exec sandbox-sh"));
    }

    #[test] // xpec: 8O
    fn developer_instructions_execute_diff_with_visible_scope() {
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
        let artifact_dir = root.join("artifacts");
        create_private_dir(&artifact_dir).unwrap();
        let mut artifact_paths = Vec::new();
        let visible_scope = vec!["scoped.txt".to_string()];

        let rendered = render_developer_instructions(
            PromptTemplateArtifactDir::Fixed(artifact_dir),
            &mut artifact_paths,
            DeveloperInstructionsContext {
                root: &root,
                mode: DeveloperInstructionsMode::GitDiff {
                    base_tree_oid: &base_tree_oid,
                    checked_tree_oid: &checked_tree_oid,
                    git_environment: &[],
                },
                question_context: "",
                visible_scope: &visible_scope,
                num_invisible_files: 1,
                last_pass: None,
            },
        )
        .unwrap();

        assert!(rendered.contains("scoped.txt"));
        assert!(rendered.contains("scoped after"));
        assert!(!rendered.contains("outside.txt"));
        assert!(!rendered.contains("outside after"));
        let _ = fs::remove_dir_all(root);
    }

    #[test] // xpec: C
    fn sh_transcript_boundary_whitespace_survives_outer_trim() {
        let output_dir = test_output_dir("sh-boundary-trim");
        let mut artifact_paths = Vec::new();

        let rendered = render_minijinja_resource_template(
            Path::new("."),
            PromptTemplateArtifactDir::Fixed(output_dir.clone()),
            &mut artifact_paths,
            " \n{% filter sh(display=\"printf kept\") %}printf '  kept\\n'{% endfilter %}\n ",
            &[],
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
        let rendered = render_developer_instructions(
            PromptTemplateArtifactDir::Fixed(output_dir.clone()),
            &mut artifact_paths,
            DeveloperInstructionsContext {
                root: Path::new("."),
                mode: if in_place {
                    DeveloperInstructionsMode::InPlace
                } else {
                    DeveloperInstructionsMode::GitDiff {
                        base_tree_oid: "HEAD",
                        checked_tree_oid: "HEAD",
                        git_environment: &[],
                    }
                },
                question_context: "Custom expectation instructions.",
                visible_scope: &visible_scope,
                num_invisible_files: 5,
                last_pass: None,
            },
        )
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

    fn run_git(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        if !output.status.success() {
            panic!(
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    #[test] // xpec: Q
    fn target_diff_prompt_hint_uses_full_q_scope_suggestion() {
        let last_pass = LastResult {
            response_timestamp: "1970-01-01T00:00:01Z".to_string(),
            updated_timestamp: "1970-01-01T00:00:01Z".to_string(),
            status: LastResultStatus::Pass,
            response: LastResultResponse::answered(
                "yes",
                "`src/a.rs`",
                Some(vec!["src/a.rs".to_string()]),
            ),
            q_scope: vec!["src/a.rs".to_string()],
            visible_scope: vec!["src/a.rs".to_string()],
            checked_tree_oid: Some("checked-tree".to_string()),
            visible_tree_oid: Some("visible-tree".to_string()),
            diff_from: None,
            diff_from_tree_oid: None,
        };
        let output_dir = test_output_dir("turn-prompt");
        let mut artifact_paths = Vec::new();

        let prompt = render_evaluator_turn_prompt(
            PromptTemplateArtifactDir::Fixed(output_dir.clone()),
            &mut artifact_paths,
            EvaluatorTurnPromptContext {
                root: Path::new("."),
                short_id: "e",
                question: "Does it pass?",
                expected_answer: "yes",
                mode: EvaluatorTurnPromptMode::GitBacked {
                    diff_from: crate::config_types::DEFAULT_DIFF_FROM,
                    render_target_diff_hint: true,
                    last_pass: Some(&last_pass),
                },
            },
        )
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

    #[test] // xpec: Q
    fn target_diff_prompt_uses_expected_answer_when_diff_from_is_not_checkpoint() {
        let last_pass = LastResult {
            response_timestamp: "1970-01-01T00:00:01Z".to_string(),
            updated_timestamp: "1970-01-01T00:00:01Z".to_string(),
            status: LastResultStatus::Pass,
            response: LastResultResponse::answered(
                "no",
                "`src/a.rs`",
                Some(vec!["src/a.rs".to_string()]),
            ),
            q_scope: vec!["src/a.rs".to_string()],
            visible_scope: vec!["src/a.rs".to_string()],
            checked_tree_oid: Some("checked-tree".to_string()),
            visible_tree_oid: Some("visible-tree".to_string()),
            diff_from: None,
            diff_from_tree_oid: None,
        };
        let output_dir = test_output_dir("turn-prompt-against-tree");
        let mut artifact_paths = Vec::new();

        let prompt = render_evaluator_turn_prompt(
            PromptTemplateArtifactDir::Fixed(output_dir.clone()),
            &mut artifact_paths,
            EvaluatorTurnPromptContext {
                root: Path::new("."),
                short_id: "e",
                question: "Does it pass?",
                expected_answer: "yes",
                mode: EvaluatorTurnPromptMode::GitBacked {
                    diff_from: crate::config_types::AGAINST_TREE_DIFF_FROM,
                    render_target_diff_hint: true,
                    last_pass: Some(&last_pass),
                },
            },
        )
        .unwrap();

        assert!(prompt.contains("This question targets the Git diff."));
        assert!(prompt.contains(r#""evidence":"""#));
        assert!(prompt.contains(r#""answer":"yes""#));
        assert!(!prompt.contains(r#""answer":"no""#));
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test] // xpec: Q
    fn in_place_turn_prompt_has_only_the_question() {
        let output_dir = test_output_dir("turn-prompt-in-place");
        let mut artifact_paths = Vec::new();

        let prompt = render_evaluator_turn_prompt(
            PromptTemplateArtifactDir::Fixed(output_dir.clone()),
            &mut artifact_paths,
            EvaluatorTurnPromptContext {
                root: Path::new("."),
                short_id: "e",
                question: "Does it pass?",
                expected_answer: "yes",
                mode: EvaluatorTurnPromptMode::InPlace,
            },
        )
        .unwrap();

        assert_eq!(prompt, r#"{"e":"Does it pass?"}"#);
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test] // xpec: C
    fn resource_template_rendering_trims_outer_whitespace() {
        let output_dir = test_output_dir("outer-trim");
        let mut artifact_paths = Vec::new();

        let rendered = render_minijinja_resource_template(
            Path::new("."),
            PromptTemplateArtifactDir::Fixed(output_dir.clone()),
            &mut artifact_paths,
            "\n  {{ value }}  \n",
            &[],
            &[],
            json!({ "value": "answer" }),
        )
        .unwrap();

        assert_eq!(rendered, "answer");
        let _ = fs::remove_dir_all(output_dir);
    }
}
