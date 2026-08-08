use super::runtime::{
    json_filter, render_with_repository_cwd, shell_args_filter, shell_quote_filter,
    trim_rendered_prompt_template_output, PromptTemplateArtifactDir, ShTranscriptMarkers,
};
use super::shell::{
    prompt_shell_filter, PromptShellContext, CONTEXT_NAME as PROMPT_SHELL_CONTEXT_NAME,
};
use minijinja::value::Value as MiniValue;
use minijinja::Environment;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// [UZ] Included resources define the static evaluator prompt and instruction
// text. Rust supplies structured values and implements template semantics such
// as the contract-defined dynamic transcript produced by the `sh` filter.
const DEVELOPER_INSTRUCTIONS_RESOURCE: &str =
    include_str!("../../../../resources/prompts/evaluator_developer_instructions.txt");
const EVALUATOR_TURN_PROMPT_RESOURCE: &str =
    include_str!("../../../../resources/prompts/evaluator_turn_prompt.txt");
pub(super) const DEVELOPER_INSTRUCTIONS_TEMPLATE_NAME: &str = "evaluator-developer-instructions";
pub(super) const EVALUATOR_TURN_PROMPT_TEMPLATE_NAME: &str = "evaluator-turn-prompt";

pub(super) struct PromptTemplateRenderRequest<'a> {
    pub(super) root: &'a Path,
    pub(super) artifact_dir: PromptTemplateArtifactDir,
    pub(super) artifact_paths: &'a mut Vec<PathBuf>,
    pub(super) shell_environment: &'a [(OsString, OsString)],
    pub(super) shell_arguments: &'a [String],
    pub(super) context: JsonValue,
}

impl<'a> PromptTemplateRenderRequest<'a> {
    pub(super) fn new(
        root: &'a Path,
        artifact_dir: PromptTemplateArtifactDir,
        artifact_paths: &'a mut Vec<PathBuf>,
        context: JsonValue,
    ) -> Self {
        PromptTemplateRenderRequest {
            root,
            artifact_dir,
            artifact_paths,
            shell_environment: &[],
            shell_arguments: &[],
            context,
        }
    }

    pub(super) fn with_shell_context(
        mut self,
        shell_environment: &'a [(OsString, OsString)],
        shell_arguments: &'a [String],
    ) -> Self {
        self.shell_environment = shell_environment;
        self.shell_arguments = shell_arguments;
        self
    }
}

pub(super) fn render_minijinja_resource_template(
    environment: &Environment<'_>,
    template_name: &str,
    request: PromptTemplateRenderRequest<'_>,
) -> Result<String, String> {
    let PromptTemplateRenderRequest {
        root,
        artifact_dir,
        artifact_paths,
        shell_environment,
        shell_arguments,
        context,
    } = request;
    let sh_transcript_markers = ShTranscriptMarkers::new()?;
    let command_artifact_paths = Arc::new(Mutex::new(Vec::new()));
    let JsonValue::Object(context) = context else {
        return Err("prompt template context must be a JSON object".to_string());
    };
    let template = environment
        .get_template(template_name)
        .map_err(|err| format!("failed to load prompt template: {}", err))?;
    // Prompt Templates require the MiniJinja render itself to start from this
    // check root cwd: the repository root outside in-place mode, or the
    // checked directory in in-place mode. The absolute root supplied by this
    // boundary also prevents shell blocks from resolving a relative root a
    // second time after the process cwd has changed.
    let rendered = render_with_repository_cwd(root, |absolute_root| {
        let shell_context = PromptShellContext::new(
            absolute_root.to_path_buf(),
            artifact_dir,
            Arc::clone(&command_artifact_paths),
            shell_environment.to_vec(),
            shell_arguments.to_vec(),
            sh_transcript_markers.clone(),
        );
        let mut render_context = context
            .into_iter()
            .map(|(key, value)| (key, MiniValue::from_serialize(value)))
            .collect::<BTreeMap<_, _>>();
        render_context.insert(
            PROMPT_SHELL_CONTEXT_NAME.to_string(),
            MiniValue::from_object(shell_context),
        );
        template.render(render_context)
    })
    .map_err(|err| format!("failed to render prompt template: {}", err))?;
    artifact_paths.extend(command_artifact_paths.lock().unwrap().iter().cloned());
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

pub(super) fn prompt_template_environment() -> Result<Environment<'static>, String> {
    let mut environment = Environment::new();
    environment.add_filter("json", json_filter);
    environment.add_filter("shq", shell_quote_filter);
    environment.add_filter("shargs", shell_args_filter);
    environment.add_filter("sh", prompt_shell_filter);
    environment
        .add_template(
            DEVELOPER_INSTRUCTIONS_TEMPLATE_NAME,
            DEVELOPER_INSTRUCTIONS_RESOURCE,
        )
        .map_err(|err| format!("failed to parse prompt template: {}", err))?;
    environment
        .add_template(
            EVALUATOR_TURN_PROMPT_TEMPLATE_NAME,
            EVALUATOR_TURN_PROMPT_RESOURCE,
        )
        .map_err(|err| format!("failed to parse prompt template: {}", err))?;
    Ok(environment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::filesystem::create_private_dir;
    use serde_json::json;
    use std::fs;

    #[test] // xpec: 3a
    fn sh_transcript_boundary_whitespace_survives_outer_trim() {
        let output_dir = test_output_dir("sh-boundary-trim");
        let mut artifact_paths = Vec::new();
        let mut environment = prompt_template_environment().unwrap();
        environment
            .add_template(
                "sh-boundary-trim",
                " \n{% filter sh(display=\"printf kept\") %}printf '  kept\\n'{% endfilter %}\n ",
            )
            .unwrap();

        let rendered = render_minijinja_resource_template(
            &environment,
            "sh-boundary-trim",
            PromptTemplateRenderRequest {
                root: Path::new("."),
                artifact_dir: PromptTemplateArtifactDir::Fixed(output_dir.clone()),
                artifact_paths: &mut artifact_paths,
                shell_environment: &[],
                shell_arguments: &[],
                context: json!({}),
            },
        )
        .unwrap();

        assert_eq!(rendered, "$ printf kept\n  kept\n");
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test] // xpec: 3a
    fn each_prompt_render_executes_its_shell_block() {
        let output_dir = test_output_dir("shell-command-execution");
        let count_path = output_dir.join("command-count");
        let mut artifact_paths = Vec::new();
        let mut environment = prompt_template_environment().unwrap();
        environment
            .add_template(
                "cached-shell-command",
                "{{ question }}\n{% filter sh(display=\"probe\") %}printf x >> \"$COUNT_FILE\"; printf output{% endfilter %}",
            )
            .unwrap();
        let shell_environment = vec![(
            OsString::from("COUNT_FILE"),
            count_path.as_os_str().to_os_string(),
        )];
        let first = render_minijinja_resource_template(
            &environment,
            "cached-shell-command",
            PromptTemplateRenderRequest {
                root: Path::new("."),
                artifact_dir: PromptTemplateArtifactDir::Fixed(output_dir.clone()),
                artifact_paths: &mut artifact_paths,
                shell_environment: &shell_environment,
                shell_arguments: &[],
                context: json!({ "question": "first" }),
            },
        )
        .unwrap();
        let second = render_minijinja_resource_template(
            &environment,
            "cached-shell-command",
            PromptTemplateRenderRequest {
                root: Path::new("."),
                artifact_dir: PromptTemplateArtifactDir::Fixed(output_dir.clone()),
                artifact_paths: &mut artifact_paths,
                shell_environment: &shell_environment,
                shell_arguments: &[],
                context: json!({ "question": "second" }),
            },
        )
        .unwrap();

        assert!(first.starts_with("first\n$ probe\noutput"));
        assert!(second.starts_with("second\n$ probe\noutput"));
        assert_eq!(fs::read_to_string(&count_path).unwrap(), "xx");
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test] // xpec: 3a
    fn relative_repository_root_is_resolved_once_for_shell_blocks() {
        let random = getrandom::u64().unwrap();
        let relative_root = PathBuf::from(format!(
            ".canon-prompt-template-relative-root-{}-{random:016x}",
            std::process::id()
        ));
        create_private_dir(&relative_root).unwrap();
        fs::write(relative_root.join("root-marker"), "relative root").unwrap();
        let output_dir = test_output_dir("relative-root-artifacts");
        let mut artifact_paths = Vec::new();
        let mut environment = prompt_template_environment().unwrap();
        environment
            .add_template(
                "relative-root",
                "{% filter sh(display=\"cat root-marker\") %}cat root-marker{% endfilter %}",
            )
            .unwrap();

        let rendered = render_minijinja_resource_template(
            &environment,
            "relative-root",
            PromptTemplateRenderRequest {
                root: &relative_root,
                artifact_dir: PromptTemplateArtifactDir::Fixed(output_dir.clone()),
                artifact_paths: &mut artifact_paths,
                shell_environment: &[],
                shell_arguments: &[],
                context: json!({}),
            },
        )
        .unwrap();

        assert_eq!(rendered, "$ cat root-marker\nrelative root\n");
        let _ = fs::remove_dir_all(relative_root);
        let _ = fs::remove_dir_all(output_dir);
    }

    #[test] // xpec: 3a
    fn resource_template_rendering_trims_outer_whitespace() {
        let output_dir = test_output_dir("outer-trim");
        let mut artifact_paths = Vec::new();
        let mut environment = prompt_template_environment().unwrap();
        environment
            .add_template("outer-trim", "\n  {{ value }}  \n")
            .unwrap();

        let rendered = render_minijinja_resource_template(
            &environment,
            "outer-trim",
            PromptTemplateRenderRequest {
                root: Path::new("."),
                artifact_dir: PromptTemplateArtifactDir::Fixed(output_dir.clone()),
                artifact_paths: &mut artifact_paths,
                shell_environment: &[],
                shell_arguments: &[],
                context: json!({ "value": "answer" }),
            },
        )
        .unwrap();

        assert_eq!(rendered, "answer");
        let _ = fs::remove_dir_all(output_dir);
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
}
