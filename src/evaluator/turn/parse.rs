use crate::check::{
    EvaluatorResponseParseError, EvaluatorResponseSchemaScope, ParsedAnswer,
    INTERNAL_ERROR_UNPARSABLE,
};
use crate::config_types::AgentConfig;
use crate::evaluator::protocol::response_cache::response_excerpt;
use crate::evaluator::EvaluatorResponseParseCache;
use std::path::Path;

pub(super) fn parse_visible_evaluator_response(
    parser_cache: &mut EvaluatorResponseParseCache,
    text: &str,
    agent: &AgentConfig,
    schema_scope: EvaluatorResponseSchemaScope,
    short_id: &str,
    answered_short_ids: &[String],
    _visible_scope: &[String],
    _session_root: &Path,
) -> Result<ParsedAnswer, EvaluatorResponseParseError> {
    // Response parsing enforces only the evaluator response schema. Evidence
    // text remains evaluator-provided justification, not check-run input.
    parser_cache.parse(text, agent, schema_scope, short_id, answered_short_ids)
}

pub(super) fn unparsable_response_answer(
    err: &EvaluatorResponseParseError,
    response: &str,
) -> ParsedAnswer {
    ParsedAnswer::error(
        INTERNAL_ERROR_UNPARSABLE.to_string(),
        format!(
            "evaluator response could not be parsed: {}\nresponse: {}",
            err,
            response_excerpt(response)
        ),
    )
}
