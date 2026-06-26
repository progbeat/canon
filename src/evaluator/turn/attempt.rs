use super::logging::{ask_and_log, LoggedTurnRequest};
use super::parse::{parse_visible_evaluator_response, unparsable_response_answer};
use super::{EvaluatorTurnContext, ParsedTurnResponse};
use crate::check::EvaluatorResponseSchemaScope;
use crate::config_types::AgentConfig;
use crate::evaluator::protocol::response_cache::response_excerpt;
use crate::evaluator::{
    EvaluatorError, EvaluatorFailureKind, EvaluatorResponseParseCache, EvaluatorRunner,
};
use crate::logs::DiagnosticLogWriter;
use serde_json::Value;
use std::path::Path;

#[allow(clippy::too_many_arguments)]
pub(crate) fn ask_once<R: EvaluatorRunner>(
    runner: &mut R,
    turn: &EvaluatorTurnContext<'_>,
    prompt: &str,
    agent: &AgentConfig,
    schema_scope: EvaluatorResponseSchemaScope,
    output_schema: &Value,
    short_id: &str,
    answered_short_ids: &[String],
    visible_scope: &[String],
    session_root: &Path,
    parser_cache: &mut EvaluatorResponseParseCache,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let response = ask_and_log(
        runner,
        diagnostic_log,
        LoggedTurnRequest {
            turn,
            prompt,
            expectation_id,
            attempt: 1,
            reason: "initial",
            output_schema,
        },
    )?;
    let (parsed, schema_valid) = match parse_visible_evaluator_response(
        parser_cache,
        &response.text,
        agent,
        schema_scope,
        short_id,
        answered_short_ids,
        visible_scope,
        session_root,
    ) {
        Ok(answer) => (answer, true),
        Err(err) if err.is_short_id_response_error() => {
            return Err(EvaluatorError::failure(
                EvaluatorFailureKind::ShortIdResponse,
                format!(
                    "evaluator response short-ID error: {}\nresponse: {}",
                    err,
                    response_excerpt(&response.text)
                ),
            ));
        }
        // Parse failures become a human-review answer. They do not trigger a
        // second evaluator request, so there is no repair request kind for the
        // progress timeline to mark.
        Err(err) => (unparsable_response_answer(&err, &response.text), false),
    };

    Ok(ParsedTurnResponse {
        answer: parsed,
        usage: response.usage,
        context_compacted: response.context_compacted,
        schema_valid,
    })
}
