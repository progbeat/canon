use minijinja::Environment;
use serde_json::json;

const EVALUATOR_BASE_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("../../../resources/prompts/evaluator_base_instructions.txt");

pub(crate) struct BaseInstructionsContext {
    pub(crate) in_place: bool,
    pub(crate) full_scope: bool,
}

pub(crate) fn evaluator_base_instructions(
    context: BaseInstructionsContext,
) -> Result<String, String> {
    let environment = Environment::new();
    let template = environment
        .template_from_str(EVALUATOR_BASE_INSTRUCTIONS_TEMPLATE)
        .map_err(|err| format!("failed to parse evaluator base instructions: {}", err))?;
    template
        .render(json!({
            "in_place": context.in_place,
            "full_scope": context.full_scope,
        }))
        .map(|rendered| rendered.trim().to_string())
        .map_err(|err| format!("failed to render evaluator base instructions: {}", err))
}

pub(crate) fn q_scope_is_full_project(scope: &[String]) -> bool {
    scope.len() == 1 && scope[0] == "."
}

#[cfg(test)]
mod tests {
    use super::{evaluator_base_instructions, BaseInstructionsContext};

    #[test]
    fn full_scope_base_instructions_do_not_mention_scope_too_narrow() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: false,
            full_scope: true,
        })
        .unwrap();

        assert!(!rendered.contains("ScopeTooNarrow"));
        assert!(rendered.contains("InvalidQuestion"));
        assert!(rendered.contains("normative text in the question itself"));
        assert!(rendered.contains("absence of a separate policy file"));
        assert!(rendered.contains("response schema includes `qScopeSuggestion`"));
        assert!(rendered.contains("transcript paths are not verification"));
        assert!(rendered.contains("Do not widen `qScopeSuggestion`"));
    }

    #[test]
    fn restricted_scope_base_instructions_allow_scope_too_narrow() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: false,
            full_scope: false,
        })
        .unwrap();

        assert!(rendered.contains("ScopeTooNarrow"));
        assert!(rendered.contains("InvalidQuestion"));
        assert!(rendered.contains("qScopeSuggestion"));
        assert!(rendered.contains("absent from the visible project"));
        assert!(rendered.contains("Answer `no` from absence"));
        assert!(rendered.contains("visible scope covers the search domain"));
    }

    #[test]
    fn git_diff_context_is_not_the_visible_tree() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: false,
            full_scope: true,
        })
        .unwrap();

        assert!(rendered.contains("not visible project files"));
        assert!(
            rendered.contains("A path shown after `full output:` is not a project-relative path")
        );
        assert!(rendered.contains("search/read visible files"));
    }

    #[test]
    fn in_place_base_instructions_do_not_use_git_diff_or_q_scope() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: true,
            full_scope: true,
        })
        .unwrap();

        assert!(!rendered.contains("Use the Git diff"));
        assert!(!rendered.contains("qScopeSuggestion"));
        assert!(!rendered.contains("sandbox transcript"));
        assert!(rendered.contains("The checked directory is the visible project."));
    }
}
