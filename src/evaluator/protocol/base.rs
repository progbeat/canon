use minijinja::Environment;
use serde_json::json;

const EVALUATOR_BASE_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("../../../resources/prompts/evaluator_base_instructions.txt");
const MAX_EVALUATOR_BASE_INSTRUCTIONS_LEN: usize = 6000;

// This template is compiled into runtime evaluator instructions; its source
// stays compact so every rendered variant satisfies the length assertion below.
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

    // xpec: w,Wg,Nb
    #[test]
    fn full_scope_base_instructions_do_not_mention_scope_too_narrow() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: false,
            full_scope: true,
        })
        .unwrap();

        // xpec: w,Wg,Nb
        assert!(!rendered.contains("ScopeTooNarrow"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("InvalidQuestion"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("substantive answer"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("Question text may itself define the specification"));
        // xpec: F
        assert!(rendered.contains("available dynamic tool output"));
        // xpec: F
        assert!(rendered.contains("cite dynamic tool output by tool name"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("do not require a separate policy file"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("`qScopeSuggestion` covers the question's search domain"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("`-` lines are removed/absent, never existing code"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("visible read/search cannot find a diff-mentioned symbol"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("derive evidence from concrete current visible project files"));
        // xpec: Q
        assert!(rendered.contains("targeting the Git diff"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("visible file conflict about current behavior"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("not active instructions or complete behavior evidence"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("Configured ignore exclusions do not make the view incomplete"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("unless the question requires an ignored path"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("as narrow as possible while still enough"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("not `InvalidQuestion` reasons"));
    }

    // xpec: w,Wg,Nb
    #[test]
    fn restricted_scope_base_instructions_allow_scope_too_narrow() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: false,
            full_scope: false,
        })
        .unwrap();

        // xpec: w,Wg,Nb
        assert!(rendered.contains("ScopeTooNarrow"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("InvalidQuestion"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("substantive answer"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("qScopeSuggestion"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("needed project path is absent"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("Restricted-scope only"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("choose `qScopeSuggestion` from the question's search domain"));
        // xpec: w,vD
        assert!(rendered.contains("chosen search domain is not contained"));
        // xpec: nJ
        assert!(!rendered.contains("do not answer project-wide absence/avoid questions"));
        // xpec: w,Wg,Nb
        assert!(!rendered.contains("Before answering, determine"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("question's search domain"));
        // xpec: nJ
        assert!(!rendered.contains("named spec/command/format compliance questions may narrow"));
        // xpec: w,vD
        assert!(rendered.contains("explicitly ranges over the whole project"));
        // xpec: w,vD
        assert!(rendered.contains("unrestricted change set"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("direct evidence, examples, and counterexamples"));
        // xpec: w,Wg,Nb
        assert!(
            rendered.contains("every path that could contain direct evidence or counterexamples")
        );
        // xpec: w,Wg,Nb
        assert!(rendered.contains("transcript relevance hints are not q-scope hiding"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("Configured ignore exclusions and transcript relevance hints"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("verify with `rg --files` or direct read/search"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("not `InvalidQuestion` reasons"));
    }

    // xpec: Q,D5,w,Wg,Nb
    #[test]
    fn git_diff_uses_are_explicit() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: false,
            full_scope: true,
        })
        .unwrap();

        // xpec: Q
        assert!(rendered.contains("retain a supplied prior answer when the diff cannot change it"));
        // xpec: D5
        assert!(rendered.contains("navigate from touched paths to current files"));
        // xpec: Q
        assert!(
            rendered.contains("may establish the diff itself only for a turn explicitly marked")
        );
        // xpec: D5
        assert!(rendered.contains("not current-behavior evidence"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("Missing diff excerpts are not missing visibility"));
        // xpec: w,Wg,Nb
        assert!(rendered.contains("read/search visible files"));
    }

    // xpec: w,Wg,Nb
    #[test]
    fn in_place_base_instructions_do_not_use_git_diff_or_q_scope() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: true,
            full_scope: true,
        })
        .unwrap();

        // xpec: w,Wg,Nb
        assert!(!rendered.contains("Use the Git diff"));
        // xpec: w,Wg,Nb
        assert!(!rendered.contains("qScopeSuggestion"));
        // xpec: w,Wg,Nb
        assert!(!rendered.contains("sandbox transcript"));
        // xpec: w,Wg,Nb
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
