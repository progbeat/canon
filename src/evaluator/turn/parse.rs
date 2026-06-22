use crate::check::{EvaluatorResponseSchemaScope, ParsedAnswer, INTERNAL_ERROR_UNPARSABLE};
use crate::config_types::AgentConfig;
use crate::evaluator::protocol::response_cache::response_excerpt;
use crate::evaluator::EvaluatorResponseParseCache;
use std::path::Path;

const RESTRICTED_RESPONSE_REPAIR_PROMPT: &str =
    include_str!("../../../resources/prompts/evaluator_restricted_repair_prompt.txt");
const FULL_PROJECT_RESPONSE_REPAIR_PROMPT: &str =
    include_str!("../../../resources/prompts/evaluator_full_project_repair_prompt.txt");

pub(super) fn response_repair_prompt(q_scope: &[String]) -> &'static str {
    match EvaluatorResponseSchemaScope::for_q_scope(q_scope) {
        EvaluatorResponseSchemaScope::Restricted => RESTRICTED_RESPONSE_REPAIR_PROMPT,
        EvaluatorResponseSchemaScope::FullProject => FULL_PROJECT_RESPONSE_REPAIR_PROMPT,
    }
}

pub(super) fn parse_visible_evaluator_response(
    parser_cache: &mut EvaluatorResponseParseCache,
    text: &str,
    agent: &AgentConfig,
    q_scope: &[String],
    _visible_scope: &[String],
    _session_root: &Path,
) -> Result<ParsedAnswer, String> {
    // Response parsing enforces only the evaluator response schema. Evidence
    // text remains evaluator-provided justification, not check-run input.
    parser_cache.parse(text, agent, q_scope)
}

pub(super) fn unparsable_response_answer(err: &str, response: &str) -> ParsedAnswer {
    ParsedAnswer::error(
        INTERNAL_ERROR_UNPARSABLE.to_string(),
        format!(
            "evaluator response could not be parsed: {}\nresponse: {}",
            err,
            response_excerpt(response)
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        response_repair_prompt, FULL_PROJECT_RESPONSE_REPAIR_PROMPT,
        RESTRICTED_RESPONSE_REPAIR_PROMPT,
    };

    #[test]
    fn repair_prompt_rejects_hidden_canon_path_citations() {
        for prompt in [
            RESTRICTED_RESPONSE_REPAIR_PROMPT,
            FULL_PROJECT_RESPONSE_REPAIR_PROMPT,
        ] {
            assert!(prompt.contains("Do not cite hidden `.canon/` paths"));
            assert!(prompt.contains("sandbox transcript"));
        }
    }

    #[test]
    fn full_project_repair_prompt_does_not_request_scope_too_narrow() {
        assert!(response_repair_prompt(&["src".to_string()]).contains("ScopeTooNarrow"));
        assert!(!response_repair_prompt(&[".".to_string()]).contains("ScopeTooNarrow"));
    }
}
