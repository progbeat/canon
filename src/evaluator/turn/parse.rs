use crate::check::{ParsedAnswer, ERROR_UNPARSABLE};
use crate::config_types::AgentConfig;
use crate::evaluator::protocol::response_cache::response_excerpt;
use crate::evaluator::EvaluatorResponseParseCache;
use std::path::Path;

pub(super) const RESPONSE_REPAIR_PROMPT: &str = "Your previous response was invalid for this same question. Return exactly one schema JSON object only, escaping quotes and backslashes inside strings. Do not include progress prose, markdown, or tool-call JSON. Cite only files visible in this evaluator working tree. Do not cite hidden `.canon/` paths, even when explaining access denial; cite the sandbox transcript or visible scope instead. If visible files are insufficient, use error:\"InsufficientEvidence\".";

pub(super) fn parse_visible_evaluator_response(
    parser_cache: &mut EvaluatorResponseParseCache,
    text: &str,
    agent: &AgentConfig,
    _visible_scope: &[String],
    _session_root: &Path,
) -> Result<ParsedAnswer, String> {
    parser_cache.parse(text, agent)
}

pub(super) fn unparsable_response_answer(err: &str, response: &str) -> ParsedAnswer {
    ParsedAnswer::error(
        ERROR_UNPARSABLE.to_string(),
        format!(
            "evaluator response could not be parsed: {}\nresponse: {}",
            err,
            response_excerpt(response)
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::RESPONSE_REPAIR_PROMPT;

    #[test]
    fn repair_prompt_rejects_hidden_canon_path_citations() {
        assert!(RESPONSE_REPAIR_PROMPT.contains("Do not cite hidden `.canon/` paths"));
        assert!(RESPONSE_REPAIR_PROMPT.contains("sandbox transcript"));
    }
}
