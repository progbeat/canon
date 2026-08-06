use crate::check::{
    EvaluatorResponseParseError, EvaluatorResponseSchemaScope, ParsedAnswer,
    INTERNAL_ERROR_UNPARSABLE,
};
use crate::evaluator::protocol::response_parse_memo::response_excerpt;
use crate::evaluator::InvocationResponseParseMemo;

pub(super) fn parse_visible_evaluator_response(
    response_parse_memo: &mut InvocationResponseParseMemo,
    text: &str,
    schema_scope: EvaluatorResponseSchemaScope,
    short_id: &str,
    answered_short_ids: &[String],
) -> Result<ParsedAnswer, EvaluatorResponseParseError> {
    // Response parsing enforces only the evaluator response schema. Evidence
    // text remains evaluator-provided justification, not check-run input.
    response_parse_memo.parse(text, schema_scope, short_id, answered_short_ids)
}

pub(super) fn unparsable_response_answer(
    err: &EvaluatorResponseParseError,
    response: &str,
) -> ParsedAnswer {
    ParsedAnswer::error_with_evidence(
        INTERNAL_ERROR_UNPARSABLE.to_string(),
        format!(
            "evaluator response could not be parsed: {}\nresponse: {}",
            err,
            response_excerpt(response)
        ),
    )
}
