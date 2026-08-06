use super::logging::{ask_and_log, LoggedTurnRequest};
use super::parse::{parse_visible_evaluator_response, unparsable_response_answer};
use super::{EvaluatorTurnContext, ParsedTurnResponse};
use crate::check::EvaluatorResponseSchemaScope;
use crate::evaluator::protocol::response_parse_memo::response_excerpt;
use crate::evaluator::{
    EvaluatorDynamicToolHandler, EvaluatorError, EvaluatorFailureKind, EvaluatorRunner,
    InvocationResponseParseMemo,
};
use crate::logs::DiagnosticLogWriter;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EvaluatorAttemptReason {
    Initial,
    ModelFallback,
    ThreadRestart,
}

impl EvaluatorAttemptReason {
    fn label(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::ModelFallback => "model-fallback",
            Self::ThreadRestart => "thread-restart",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EvaluatorAttempt {
    number: usize,
    reason: EvaluatorAttemptReason,
}

#[derive(Default)]
pub(crate) struct EvaluatorAttemptSequence {
    issued: usize,
}

impl EvaluatorAttemptSequence {
    pub(crate) fn next(&mut self, reason: EvaluatorAttemptReason) -> EvaluatorAttempt {
        self.issued += 1;
        EvaluatorAttempt {
            number: self.issued,
            reason,
        }
    }
}

pub(crate) struct EvaluatorAttemptRequest<'a> {
    pub(crate) attempt: EvaluatorAttempt,
    pub(crate) turn: &'a EvaluatorTurnContext<'a>,
    pub(crate) task_input: &'a str,
    pub(crate) schema_scope: EvaluatorResponseSchemaScope,
    pub(crate) output_schema: &'a Value,
    pub(crate) short_id: &'a str,
    pub(crate) answered_short_ids: &'a [String],
    pub(crate) expectation_id: Option<&'a str>,
}

pub(crate) fn ask_once<R: EvaluatorRunner>(
    runner: &mut R,
    response_parse_memo: &mut InvocationResponseParseMemo,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    request: EvaluatorAttemptRequest<'_>,
    dynamic_tool_handler: Option<&mut dyn EvaluatorDynamicToolHandler>,
) -> Result<ParsedTurnResponse, EvaluatorError> {
    let response = ask_and_log(
        runner,
        diagnostic_log,
        LoggedTurnRequest {
            turn: request.turn,
            task_input: request.task_input,
            expectation_id: request.expectation_id,
            attempt: request.attempt.number,
            reason: request.attempt.reason.label(),
            output_schema: request.output_schema,
        },
        dynamic_tool_handler,
    )?;
    // One evaluator turn asks for exactly one interrogation short ID. The
    // parser rejects missing, already-answered, or extra short IDs to enforce
    // the Interrogation Policy for that single requested interrogation. If a
    // future check turn batches multiple requested interrogations, this
    // boundary must change from `short_id` to the requested short-ID set.
    let (parsed, schema_valid) = match parse_visible_evaluator_response(
        response_parse_memo,
        &response.text,
        request.schema_scope,
        request.short_id,
        request.answered_short_ids,
    ) {
        Ok(answer) => (answer, true),
        // [qv] A ShortIdResponse failure means this evaluator thread already
        // produced a valid turn. The lifecycle can therefore interpret this
        // failure kind as an unconditional fresh-thread retry request.
        Err(err) if err.is_short_id_response_error() && !request.answered_short_ids.is_empty() => {
            return Err(EvaluatorError::failure(
                EvaluatorFailureKind::ShortIdResponse,
                format!(
                    "evaluator response short-ID error: {}\nresponse: {}",
                    err,
                    response_excerpt(&response.text)
                ),
            ));
        }
        // [qv,w] A first-turn short-ID mismatch and all other parse failures
        // become a human-review answer without a thread retry. This turn
        // boundary has already logged the raw exchange; it deliberately does
        // not claim the final expectation outcome. Normal check execution
        // converts the answer to a CheckRecord, then
        // `check::engine::execute::expectation::finish` writes both the
        // `expectation.result` and applicable `expectation.review_required`
        // runtime events. A parse failure does not trigger a second evaluator
        // request, so there is no repair request kind for the timeline to mark.
        Err(err) => (unparsable_response_answer(&err, &response.text), false),
    };

    Ok(ParsedTurnResponse {
        answer: parsed,
        context_compacted: response.context_compacted,
        schema_valid,
    })
}

#[cfg(test)]
mod tests {
    use super::{EvaluatorAttempt, EvaluatorAttemptReason, EvaluatorAttemptSequence};

    #[test] // xpec: gN
    fn attempt_sequence_numbers_distinct_retry_reasons() {
        let mut attempts = EvaluatorAttemptSequence::default();

        assert_eq!(
            attempts.next(EvaluatorAttemptReason::Initial),
            EvaluatorAttempt {
                number: 1,
                reason: EvaluatorAttemptReason::Initial,
            }
        );
        assert_eq!(
            attempts.next(EvaluatorAttemptReason::ThreadRestart),
            EvaluatorAttempt {
                number: 2,
                reason: EvaluatorAttemptReason::ThreadRestart,
            }
        );
        assert_eq!(
            attempts.next(EvaluatorAttemptReason::ModelFallback),
            EvaluatorAttempt {
                number: 3,
                reason: EvaluatorAttemptReason::ModelFallback,
            }
        );
    }
}
