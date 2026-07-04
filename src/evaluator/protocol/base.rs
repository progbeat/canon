use minijinja::Environment;
use serde_json::json;

const EVALUATOR_BASE_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("../../../resources/prompts/evaluator_base_instructions.txt");
const MAX_EVALUATOR_BASE_INSTRUCTIONS_LEN: usize = 6000;

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
    let rendered = template
        .render(json!({
            "in_place": context.in_place,
            "full_scope": context.full_scope,
        }))
        .map(|rendered| rendered.trim().to_string())
        .map_err(|err| format!("failed to render evaluator base instructions: {}", err))?;
    // xpec: Uy
    assert!(
        rendered.len() <= MAX_EVALUATOR_BASE_INSTRUCTIONS_LEN,
        "evaluator base instructions rendered length {} exceeds {}",
        rendered.len(),
        MAX_EVALUATOR_BASE_INSTRUCTIONS_LEN
    );
    Ok(rendered)
}

pub(crate) fn q_scope_is_full_project(scope: &[String]) -> bool {
    scope.len() == 1 && scope[0] == "."
}

#[cfg(test)]
mod tests {
    use super::{
        evaluator_base_instructions, BaseInstructionsContext, MAX_EVALUATOR_BASE_INSTRUCTIONS_LEN,
    };

    // xpec: 92,Wg,Nb
    #[test]
    fn full_scope_base_instructions_do_not_mention_scope_too_narrow() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: false,
            full_scope: true,
        })
        .unwrap();

        // xpec: 92,Wg,Nb
        assert!(!rendered.contains("ScopeTooNarrow"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("InvalidQuestion"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("substantive answer"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("Question text may itself define the specification"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("do not require a separate policy file"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("`qScopeSuggestion` covers the question's search domain"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains(
            "diff transcript lines, transcript paths, and removed diff lines are not verification"
        ));
        // xpec: 92,Wg,Nb
        assert!(rendered
            .contains("not the diff, changed-file lists, or absence of relevant changed files"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("not active instructions or complete behavior evidence"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("Configured ignore exclusions do not make the view incomplete"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("unless the question requires an ignored path"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("as narrow as possible while still enough"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("not `InvalidQuestion` reasons"));
    }

    // xpec: 92,Wg,Nb
    #[test]
    fn restricted_scope_base_instructions_allow_scope_too_narrow() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: false,
            full_scope: false,
        })
        .unwrap();

        // xpec: 92,Wg,Nb
        assert!(rendered.contains("ScopeTooNarrow"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("InvalidQuestion"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("substantive answer"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("qScopeSuggestion"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("needed project path is absent"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("Restricted-scope only"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("choose `qScopeSuggestion` from the question's search domain"));
        // xpec: YD,v
        assert!(rendered.contains("chosen search domain is not contained"));
        // xpec: YD,v
        assert!(rendered.contains("do not answer project-wide absence/avoid questions"));
        // xpec: 92,Wg,Nb
        assert!(!rendered.contains("Before answering, determine"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("question's search domain"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("implementation/spec compliance"));
        // xpec: v
        assert!(rendered.contains("project-wide quality/safety/dead-code"));
        // xpec: v
        assert!(rendered.contains("project-wide \"find any\" or \"avoid any\" questions"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("change-set"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("direct evidence, examples, and counterexamples"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("paths that could contain direct evidence or counterexamples"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("Concrete behavior questions may narrow"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("transcript relevance hints are not q-scope hiding"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("Configured ignore exclusions and transcript relevance hints"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("verify with `rg --files` or direct read/search"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("not `InvalidQuestion` reasons"));
    }

    // xpec: 92,Wg,Nb
    #[test]
    fn git_diff_context_is_not_the_visible_tree() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: false,
            full_scope: true,
        })
        .unwrap();

        // xpec: 92,Wg,Nb
        assert!(
            rendered.contains("diff transcript/full-output files are navigation, never evidence")
        );
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("Missing diff excerpts are not missing visibility"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("read/search visible files"));
    }

    // xpec: 92,Wg,Nb
    #[test]
    fn in_place_base_instructions_do_not_use_git_diff_or_q_scope() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: true,
            full_scope: true,
        })
        .unwrap();

        // xpec: 92,Wg,Nb
        assert!(!rendered.contains("Use the Git diff"));
        // xpec: 92,Wg,Nb
        assert!(!rendered.contains("qScopeSuggestion"));
        // xpec: 92,Wg,Nb
        assert!(!rendered.contains("sandbox transcript"));
        // xpec: 92,Wg,Nb
        assert!(rendered.contains("The checked directory is the visible project."));
    }

    // xpec: Uy
    #[test]
    fn base_instructions_render_within_length_limit() {
        for context in [
            BaseInstructionsContext {
                in_place: false,
                full_scope: true,
            },
            BaseInstructionsContext {
                in_place: false,
                full_scope: false,
            },
            BaseInstructionsContext {
                in_place: true,
                full_scope: true,
            },
            BaseInstructionsContext {
                in_place: true,
                full_scope: false,
            },
        ] {
            let rendered = evaluator_base_instructions(context).unwrap();
            // xpec: Uy
            assert!(rendered.len() <= MAX_EVALUATOR_BASE_INSTRUCTIONS_LEN);
        }
    }
}
