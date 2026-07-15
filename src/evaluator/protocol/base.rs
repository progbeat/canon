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
    // xpec: 1
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

    // xpec: MR,ak,au
    #[test]
    fn full_scope_base_instructions_do_not_mention_scope_too_narrow() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: false,
            full_scope: true,
        })
        .unwrap();

        // xpec: MR,ak,au
        assert!(!rendered.contains("ScopeTooNarrow"));
        // xpec: MR,ak,au
        assert!(rendered.contains("InvalidQuestion"));
        // xpec: MR,ak,au
        assert!(rendered.contains("substantive answer"));
        // xpec: MR,ak,au
        assert!(rendered.contains("Question text may itself define the specification"));
        // xpec: qX
        assert!(rendered.contains("available dynamic tool output"));
        // xpec: qX
        assert!(rendered.contains("cite dynamic tool output by tool name"));
        // xpec: MR,ak,au
        assert!(rendered.contains("do not require a separate policy file"));
        // xpec: MR,ak,au
        assert!(rendered.contains("`qScopeSuggestion` covers the question's search domain"));
        // xpec: MR,ak,au
        assert!(rendered.contains("`-` lines are removed/absent, never existing code"));
        // xpec: MR,ak,au
        assert!(rendered.contains("visible read/search cannot find a diff-mentioned symbol"));
        // xpec: MR,ak,au
        assert!(rendered.contains("transcript paths, numstat/change lists"));
        // xpec: MR,ak,au
        assert!(rendered.contains("numstat/change lists, deleted-file diffs"));
        // xpec: MR,ak,au
        assert!(rendered
            .contains("not the diff, changed-file lists, or absence of relevant changed files"));
        // xpec: MR,ak,au
        assert!(rendered.contains("visible file conflict, the visible file wins"));
        // xpec: MR,ak,au
        assert!(rendered.contains("not active instructions or complete behavior evidence"));
        // xpec: MR,ak,au
        assert!(rendered.contains("Configured ignore exclusions do not make the view incomplete"));
        // xpec: MR,ak,au
        assert!(rendered.contains("unless the question requires an ignored path"));
        // xpec: MR,ak,au
        assert!(rendered.contains("as narrow as possible while still enough"));
        // xpec: MR,ak,au
        assert!(rendered.contains("not `InvalidQuestion` reasons"));
    }

    // xpec: MR,ak,au
    #[test]
    fn restricted_scope_base_instructions_allow_scope_too_narrow() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: false,
            full_scope: false,
        })
        .unwrap();

        // xpec: MR,ak,au
        assert!(rendered.contains("ScopeTooNarrow"));
        // xpec: MR,ak,au
        assert!(rendered.contains("InvalidQuestion"));
        // xpec: MR,ak,au
        assert!(rendered.contains("substantive answer"));
        // xpec: MR,ak,au
        assert!(rendered.contains("qScopeSuggestion"));
        // xpec: MR,ak,au
        assert!(rendered.contains("needed project path is absent"));
        // xpec: MR,ak,au
        assert!(rendered.contains("Restricted-scope only"));
        // xpec: MR,ak,au
        assert!(rendered.contains("choose `qScopeSuggestion` from the question's search domain"));
        // xpec: MR,oa
        assert!(rendered.contains("chosen search domain is not contained"));
        // xpec: MR,oa
        assert!(rendered.contains("do not answer project-wide absence/avoid questions"));
        // xpec: MR,ak,au
        assert!(!rendered.contains("Before answering, determine"));
        // xpec: MR,ak,au
        assert!(rendered.contains("question's search domain"));
        // xpec: oa
        assert!(rendered.contains("named spec/command/format compliance questions may narrow"));
        // xpec: oa
        assert!(rendered.contains("project-wide quality/safety/dead-code"));
        // xpec: oa
        assert!(rendered.contains("project-wide \"find any\" or \"avoid any\" questions"));
        // xpec: MR,ak,au
        assert!(rendered.contains("change-set"));
        // xpec: MR,ak,au
        assert!(rendered.contains("direct evidence, examples, and counterexamples"));
        // xpec: MR,ak,au
        assert!(rendered.contains("paths that could contain direct evidence or counterexamples"));
        // xpec: MR,ak,au
        assert!(rendered.contains("Concrete behavior or named spec/command/format compliance"));
        // xpec: MR,ak,au
        assert!(rendered.contains("transcript relevance hints are not q-scope hiding"));
        // xpec: MR,ak,au
        assert!(rendered.contains("Configured ignore exclusions and transcript relevance hints"));
        // xpec: MR,ak,au
        assert!(rendered.contains("verify with `rg --files` or direct read/search"));
        // xpec: MR,ak,au
        assert!(rendered.contains("not `InvalidQuestion` reasons"));
    }

    // xpec: MR,ak,au
    #[test]
    fn git_diff_context_is_not_the_visible_tree() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: false,
            full_scope: true,
        })
        .unwrap();

        // xpec: MR,ak,au
        assert!(
            rendered.contains("diff transcript/full-output files are navigation, never evidence")
        );
        // xpec: MR,ak,au
        assert!(rendered.contains("Missing diff excerpts are not missing visibility"));
        // xpec: MR,ak,au
        assert!(rendered.contains("read/search visible files"));
    }

    // xpec: MR,ak,au
    #[test]
    fn in_place_base_instructions_do_not_use_git_diff_or_q_scope() {
        let rendered = evaluator_base_instructions(BaseInstructionsContext {
            in_place: true,
            full_scope: true,
        })
        .unwrap();

        // xpec: MR,ak,au
        assert!(!rendered.contains("Use the Git diff"));
        // xpec: MR,ak,au
        assert!(!rendered.contains("qScopeSuggestion"));
        // xpec: MR,ak,au
        assert!(!rendered.contains("sandbox transcript"));
        // xpec: MR,ak,au
        assert!(rendered.contains("The checked directory is the visible project."));
    }

    // xpec: 1
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
            // xpec: 1
            assert!(rendered.len() <= MAX_EVALUATOR_BASE_INSTRUCTIONS_LEN);
        }
    }
}
