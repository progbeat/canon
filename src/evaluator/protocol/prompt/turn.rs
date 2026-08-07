use super::runtime::PromptTemplateArtifactDir;
use super::template::{
    render_minijinja_resource_template, PromptTemplateRenderRequest,
    EVALUATOR_TURN_PROMPT_TEMPLATE_NAME,
};
use super::EvaluatorTurnPromptContext;
use minijinja::Environment;
use serde_json::json;
use std::path::PathBuf;

pub(super) fn render_evaluator_turn_prompt_with_artifacts(
    environment: &Environment<'_>,
    template_artifact_dir: PromptTemplateArtifactDir,
    template_artifact_paths: &mut Vec<PathBuf>,
    context: EvaluatorTurnPromptContext<'_>,
) -> Result<String, String> {
    // [UZ] This function supplies structured values. The included resource
    // defines the static text, and the shared renderer applies template and
    // filter semantics.
    // [X,hj] For a diff target, `only` scopes the subject being evaluated to
    // the diff-affected files; the canonical hint explicitly keeps every
    // other visible file available as context.
    render_minijinja_resource_template(
        environment,
        EVALUATOR_TURN_PROMPT_TEMPLATE_NAME,
        PromptTemplateRenderRequest::new(
            context.root,
            template_artifact_dir,
            template_artifact_paths,
            json!({
                "xpec": {
                    "short_id": context.short_id,
                    "q": context.question,
                    "target": if context.mode.target_is_diff() { "diff" } else { "" },
                },
            }),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::super::{EvaluatorPromptMode, EvaluatorTurnPromptContext, PromptRenderer};
    use std::path::Path;

    fn render(mode: EvaluatorPromptMode<'_>) -> String {
        PromptRenderer::new(crate::platform::filesystem::PrivateTemporaryDirectoryAllocator::new())
            .evaluator_turn_prompt(EvaluatorTurnPromptContext {
                root: Path::new("."),
                short_id: "e",
                question: "Does it pass?",
                mode,
            })
            .unwrap()
            .text
    }

    #[test] // xpec: X,hj
    fn target_diff_turn_prompt_assigns_distinct_evaluation_and_context_roles() {
        let prompt = render(EvaluatorPromptMode::GitDiff {
            target_is_diff: true,
            base_tree_oid: "HEAD",
            checked_tree_oid: "HEAD",
            git_environment: &[],
        });

        // [X,hj] One canonical instruction assigns affected files as the
        // evaluation subject and every other visible file as context; neither
        // role changes or qualifies the other.
        assert_eq!(
            prompt,
            "{\"e\":\"Does it pass?\"}\n\
             Evaluate only the visible files affected by the diff; use other visible files as context!"
        );
    }

    #[test] // xpec: X
    fn project_turn_prompt_has_only_the_question() {
        let prompt = render(EvaluatorPromptMode::GitDiff {
            target_is_diff: false,
            base_tree_oid: "HEAD",
            checked_tree_oid: "HEAD",
            git_environment: &[],
        });
        assert_eq!(prompt, r#"{"e":"Does it pass?"}"#);
    }

    #[test] // xpec: X
    fn in_place_turn_prompt_has_only_the_question() {
        assert_eq!(
            render(EvaluatorPromptMode::InPlace),
            r#"{"e":"Does it pass?"}"#
        );
    }
}
