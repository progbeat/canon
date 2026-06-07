use super::logging::ask_and_log;
use super::parse::{
    parse_visible_evaluator_response, unparsable_response_answer, RESPONSE_REPAIR_PROMPT,
};
use super::{EvaluatorTurnContext, ParsedTurnResponse};
use crate::config_types::AgentConfig;
use crate::evaluator::{EvaluatorError, EvaluatorResponseParseCache, EvaluatorRunner};
use crate::logs::DiagnosticLogWriter;
use crate::token_usage_types::TokenUsage;
use std::path::Path;

pub(crate) fn ask_once<R: EvaluatorRunner>(
    runner: &mut R,
    turn: &EvaluatorTurnContext<'_>,
    prompt: &str,
    agent: &AgentConfig,
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
    )?;
    let mut usage = response.usage;
    let mut context_compacted = response.context_compacted;
    let parsed = match parse_visible_evaluator_response(
        parser_cache,
        &response.text,
        agent,
        visible_scope,
        session_root,
    ) {
        Ok(answer) => answer,
        Err(_) => {
            let repair = ask_and_log(
                runner,
                turn,
                RESPONSE_REPAIR_PROMPT,
                diagnostic_log,
                expectation_id,
                2,
                "repair",
            )?;
            usage = combined_turn_usage(usage, repair.usage);
            context_compacted |= repair.context_compacted;
            match parse_visible_evaluator_response(
                parser_cache,
                &repair.text,
                agent,
                visible_scope,
                session_root,
            ) {
                Ok(answer) => answer,
                Err(err) => unparsable_response_answer(&err, &repair.text),
            }
        }
    };

    Ok(ParsedTurnResponse {
        answer: parsed,
        usage,
        context_compacted,
    })
}

fn combined_turn_usage(
    first: Option<TokenUsage>,
    second: Option<TokenUsage>,
) -> Option<TokenUsage> {
    match (first, second) {
        (Some(first), Some(second)) => Some(first.add(second)),
        (Some(usage), None) | (None, Some(usage)) => Some(usage),
        (None, None) => None,
    }
}
