use crate::check::{ParsedAnswer, ERROR_INSUFFICIENT_EVIDENCE, ERROR_UNPARSABLE};
use crate::config_types::AgentConfig;
use crate::evaluator::response_cache::{response_excerpt, EvaluatorResponseParseCache};
use crate::evidence::evidence_file_refs_are_visible_in_root;
use std::path::Path;

pub(super) const RESPONSE_REPAIR_PROMPT: &str = "Your previous response was invalid for this same question. Return exactly one schema JSON object only, escaping quotes and backslashes inside strings. Do not include progress prose, markdown, or tool-call JSON. Cite only files visible in this evaluator working tree; if visible files are insufficient, use error:\"insufficient-evidence\".";

pub(super) fn parse_visible_evaluator_response(
    parser_cache: &mut EvaluatorResponseParseCache,
    text: &str,
    agent: &AgentConfig,
    visible_scope: &[String],
    session_root: Option<&Path>,
) -> Result<ParsedAnswer, EvaluatorResponseParseError> {
    let answer = parser_cache
        .parse(text, agent)
        .map_err(EvaluatorResponseParseError::InvalidResponse)?;
    if evidence_file_refs_are_visible_in_root(&answer.evidence, visible_scope, session_root) {
        Ok(answer)
    } else {
        Err(EvaluatorResponseParseError::OutOfScopeEvidence)
    }
}

pub(super) enum EvaluatorResponseParseError {
    InvalidResponse(String),
    OutOfScopeEvidence,
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

pub(super) fn insufficient_evidence_response_answer() -> ParsedAnswer {
    ParsedAnswer::error(
        ERROR_INSUFFICIENT_EVIDENCE.to_string(),
        "evaluator evidence cites files outside the visible scope".to_string(),
    )
}
