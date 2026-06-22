use super::logging::ask_and_log;
use super::parse::{parse_visible_evaluator_response, unparsable_response_answer};
use super::{EvaluatorTurnContext, ParsedTurnResponse};
use crate::config_types::AgentConfig;
use crate::evaluator::{EvaluatorError, EvaluatorResponseParseCache, EvaluatorRunner};
use crate::logs::DiagnosticLogWriter;
use std::path::Path;

#[allow(clippy::too_many_arguments)]
pub(crate) fn ask_once<R: EvaluatorRunner>(
    runner: &mut R,
    turn: &EvaluatorTurnContext<'_>,
    prompt: &str,
    agent: &AgentConfig,
    q_scope: &[String],
    visible_scope: &[String],
    session_root: &Path,
    parser_cache: &mut EvaluatorResponseParseCache,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let response = ask_and_log(
        runner,
        turn,
        prompt,
        diagnostic_log,
        expectation_id,
        1,
        "initial",
        q_scope,
    )?;
    let parsed = match parse_visible_evaluator_response(
        parser_cache,
        &response.text,
        agent,
        q_scope,
        visible_scope,
        session_root,
    ) {
        Ok(answer) => answer,
        Err(err) => unparsable_response_answer(&err, &response.text),
    };

    Ok(ParsedTurnResponse {
        answer: parsed,
        usage: response.usage,
        context_compacted: response.context_compacted,
    })
}
