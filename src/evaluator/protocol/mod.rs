pub(crate) mod response_parse_memo;

mod base;
mod dynamic_tools;
mod prompt;
mod prompt_artifact_permissions;
mod prompt_shell;
mod types;

pub(crate) use base::{evaluator_base_instructions, BaseInstructionsContext};
pub(crate) use dynamic_tools::canon_show_dynamic_tools;
pub(crate) use prompt::{
    developer_instructions_cache_key, DeveloperInstructionsCacheKey, DeveloperInstructionsContext,
    EvaluatorPromptMode, EvaluatorTurnPromptContext, PromptRenderer, RenderedPrompt,
};
pub(crate) use response_parse_memo::InvocationResponseParseMemo;
pub(crate) use types::{
    EvaluatorDynamicToolCall, EvaluatorDynamicToolHandler, EvaluatorDynamicToolResult,
    EvaluatorError, EvaluatorRunner,
};

#[cfg(test)]
mod instruction_set_tests {
    use super::*;
    use std::path::Path;

    #[test] // xpec: X,hj,Ez
    fn diff_subject_keeps_context_distinct_from_the_invalidation_boundary() {
        let base = evaluator_base_instructions(BaseInstructionsContext {
            in_place: false,
            q_scope_is_full_project: false,
            q_scope_is_auto: true,
            q_scope_verification: false,
        })
        .unwrap();
        let turn = PromptRenderer::new(
            crate::platform::filesystem::PrivateTemporaryDirectoryAllocator::new(),
        )
        .evaluator_turn_prompt(EvaluatorTurnPromptContext {
            root: Path::new("."),
            short_id: "e",
            question: "Does it pass?",
            mode: EvaluatorPromptMode::GitDiff {
                target_is_diff: true,
                base_tree_oid: "HEAD",
                checked_tree_oid: "HEAD",
                git_environment: &[],
            },
        })
        .unwrap()
        .text;

        assert!(base.contains("future cache-invalidation boundary, not a record of files visible"));
        assert!(base.contains("narrow self-contained affected owning boundaries"));
        assert!(turn.contains("use other visible files as context"));
    }
}
