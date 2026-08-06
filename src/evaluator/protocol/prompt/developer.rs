use super::runtime::PromptTemplateArtifactDir;
use super::template::{
    render_minijinja_resource_template, PromptTemplateRenderRequest,
    DEVELOPER_INSTRUCTIONS_TEMPLATE_NAME,
};
use super::{git_diff_shell_environment, DeveloperInstructionsContext, EvaluatorPromptMode};
use minijinja::Environment;
use serde_json::json;
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct DeveloperInstructionsCacheKey {
    root: PathBuf,
    mode: DeveloperInstructionsCacheMode,
    question_context: String,
    visible_scope: Vec<String>,
    num_invisible_files: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum DeveloperInstructionsCacheMode {
    InPlace,
    GitDiff {
        target_is_diff: bool,
        base_tree_oid: String,
        checked_tree_oid: String,
        git_environment: Vec<(OsString, OsString)>,
    },
}

pub(crate) fn developer_instructions_cache_key(
    context: &DeveloperInstructionsContext<'_>,
) -> DeveloperInstructionsCacheKey {
    let mode = match context.mode {
        EvaluatorPromptMode::InPlace => DeveloperInstructionsCacheMode::InPlace,
        EvaluatorPromptMode::GitDiff {
            target_is_diff,
            base_tree_oid,
            checked_tree_oid,
            git_environment,
        } => DeveloperInstructionsCacheMode::GitDiff {
            target_is_diff,
            base_tree_oid: base_tree_oid.to_string(),
            checked_tree_oid: checked_tree_oid.to_string(),
            git_environment: git_environment.to_vec(),
        },
    };
    DeveloperInstructionsCacheKey {
        root: context.root.to_path_buf(),
        mode,
        question_context: context.question_context.to_string(),
        visible_scope: context.visible_scope.to_vec(),
        num_invisible_files: context.num_invisible_files,
    }
}

pub(super) fn render_developer_instructions_with_artifacts(
    environment: &Environment<'_>,
    template_artifact_dir: PromptTemplateArtifactDir,
    template_artifact_paths: &mut Vec<PathBuf>,
    context: DeveloperInstructionsContext<'_>,
) -> Result<String, String> {
    // xpec: Ka
    // Each `sh` block gets the canonical visible scope as positional shell
    // arguments. The resource can therefore execute its specified `"$@"`
    // commands even though template filters run independently.
    let (in_place, git_diff_environment) = match context.mode {
        EvaluatorPromptMode::InPlace => (true, Vec::new()),
        EvaluatorPromptMode::GitDiff {
            base_tree_oid,
            checked_tree_oid,
            git_environment,
            ..
        } => (
            false,
            git_diff_shell_environment(base_tree_oid, checked_tree_oid, git_environment),
        ),
    };
    render_minijinja_resource_template(
        environment,
        DEVELOPER_INSTRUCTIONS_TEMPLATE_NAME,
        PromptTemplateRenderRequest::new(
            context.root,
            template_artifact_dir,
            template_artifact_paths,
            json!({
                "xpec": {
                    // [UZ] This is human-authored expectation context rendered by
                    // the resource template, not another implementation-owned
                    // evaluator prompt or instruction source.
                    "instructions": context.question_context,
                    "target": if context.mode.target_is_diff() { "diff" } else { "" },
                    "visible_scope": context.visible_scope,
                },
                "in_place": in_place,
                "num_invisible_files": context.num_invisible_files,
                // The canonical full q-scope normalizes to "." before
                // configured exclusions are appended to the visible scope.
                "full_scope": context.visible_scope.first().is_some_and(|path| path == "."),
            }),
        )
        .with_shell_context(&git_diff_environment, context.visible_scope),
    )
}

#[cfg(test)]
mod tests;
