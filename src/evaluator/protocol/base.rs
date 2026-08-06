use minijinja::Environment;
use serde_json::json;
use std::sync::OnceLock;

// [UZ] This module parses, renders, and caches resource-owned text. The
// `include_str!` path below identifies its sole defining source; no
// evaluator-facing wording is assembled in Rust.
const EVALUATOR_BASE_INSTRUCTIONS_RESOURCE: &str =
    include_str!("../../../resources/prompts/evaluator_base_instructions.txt");
const EVALUATOR_BASE_INSTRUCTIONS_TEMPLATE_NAME: &str = "evaluator-base-instructions";
const MAX_EVALUATOR_BASE_INSTRUCTIONS_LEN: usize = 6000;
static PARSED_BASE_INSTRUCTIONS_ENVIRONMENT: OnceLock<Result<Environment<'static>, String>> =
    OnceLock::new();
static RENDERED_BASE_INSTRUCTIONS_BY_CONTEXT: [OnceLock<Result<String, String>>; 16] =
    [const { OnceLock::new() }; 16];

// These values provide only render context for the resource template.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct BaseInstructionsContext {
    pub(crate) in_place: bool,
    // This controls q-scope visibility, independently of the expectation target.
    pub(crate) q_scope_is_full_project: bool,
    pub(crate) q_scope_is_auto: bool,
    pub(crate) q_scope_verification: bool,
}

impl BaseInstructionsContext {
    fn rendered_instructions_index(self) -> usize {
        usize::from(self.in_place) << 3
            | usize::from(self.q_scope_is_full_project) << 2
            | usize::from(self.q_scope_is_auto) << 1
            | usize::from(self.q_scope_verification)
    }
}

pub(crate) fn evaluator_base_instructions(
    context: BaseInstructionsContext,
) -> Result<String, String> {
    // [d] The static template and these four booleans are the complete
    // renderer input domain; this component performs no repository or
    // filesystem inspection. Cache each of the 2^4 rendered results/errors.
    RENDERED_BASE_INSTRUCTIONS_BY_CONTEXT[context.rendered_instructions_index()]
        .get_or_init(|| render_evaluator_base_instructions(context))
        .clone()
}

fn render_evaluator_base_instructions(context: BaseInstructionsContext) -> Result<String, String> {
    let environment = match PARSED_BASE_INSTRUCTIONS_ENVIRONMENT.get_or_init(|| {
        let mut environment = Environment::new();
        environment
            .add_template(
                EVALUATOR_BASE_INSTRUCTIONS_TEMPLATE_NAME,
                EVALUATOR_BASE_INSTRUCTIONS_RESOURCE,
            )
            .map_err(|err| format!("failed to parse evaluator base instructions: {}", err))?;
        Ok(environment)
    }) {
        Ok(environment) => environment,
        Err(error) => return Err(error.clone()),
    };
    let template = environment
        .get_template(EVALUATOR_BASE_INSTRUCTIONS_TEMPLATE_NAME)
        .map_err(|err| format!("failed to load evaluator base instructions: {}", err))?;
    let rendered = template
        .render(json!({
            "in_place": context.in_place,
            "q_scope_is_full_project": context.q_scope_is_full_project,
            "q_scope_is_auto": context.q_scope_is_auto,
            "q_scope_verification": context.q_scope_verification,
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

#[cfg(test)]
mod tests {
    use super::{
        evaluator_base_instructions, BaseInstructionsContext, MAX_EVALUATOR_BASE_INSTRUCTIONS_LEN,
    };

    fn render(
        in_place: bool,
        q_scope_is_full_project: bool,
        q_scope_is_auto: bool,
        q_scope_verification: bool,
    ) -> String {
        evaluator_base_instructions(BaseInstructionsContext {
            in_place,
            q_scope_is_full_project,
            q_scope_is_auto,
            q_scope_verification,
        })
        .unwrap()
    }

    #[test] // xpec: qv,hj
    fn q_scope_context_selects_resource_owned_contract_branches() {
        let full_project_auto = render(false, true, true, false);
        let fixed_full_project = render(false, true, false, false);
        let fixed_restricted = render(false, false, false, false);
        let auto_restricted = render(false, false, true, false);

        assert_ne!(full_project_auto, fixed_full_project);
        assert_eq!(fixed_full_project, fixed_restricted);
        assert_ne!(full_project_auto, auto_restricted);
        assert!(full_project_auto.contains("future cache-invalidation boundary"));
        assert!(!fixed_full_project.contains("future cache-invalidation boundary"));
    }

    #[test] // xpec: qv,hj
    fn in_place_rendering_is_independent_of_q_scope_context() {
        let baseline = render(true, true, true, false);
        for q_scope_is_full_project in [false, true] {
            for q_scope_is_auto in [false, true] {
                for q_scope_verification in [false, true] {
                    assert_eq!(
                        baseline,
                        render(
                            true,
                            q_scope_is_full_project,
                            q_scope_is_auto,
                            q_scope_verification
                        )
                    );
                }
            }
        }
    }

    #[test] // xpec: qv,Ez,hj
    fn verification_context_changes_restricted_auto_instructions() {
        let initial = render(false, false, true, false);
        let verification = render(false, false, true, true);

        assert_ne!(verification, initial);
    }
}
