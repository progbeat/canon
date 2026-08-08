//! Shared check-domain model and representation contracts.
//!
//! This component owns the values passed between check configuration,
//! evaluation, persistence, and output. Those workflows live in their own
//! components; the modules here define the common xpec, response, record,
//! report, and command-option vocabulary at their boundary.

mod answer;
pub(super) mod errors;
mod evaluation_answer;
mod evaluator_response;
mod expectation;
mod line_break;
mod options;
mod record;
mod run_report;

pub(crate) use answer::{assert_evaluation_postconditions, evaluate_final_response, CheckResult};
pub(crate) use errors::INTERNAL_ERROR_UNPARSABLE;
pub(crate) use evaluation_answer::EvaluationAnswer;
pub(crate) use evaluator_response::{
    evaluator_response_output_schema_for_scope, matches_answer_pattern,
    parse_evaluator_response_for_short_id, EvaluatorResponseParseError,
    EvaluatorResponseSchemaScope, ParsedAnswer, ANSWER_PATTERN, ERROR_INVALID_QUESTION,
    ERROR_SCOPE_TOO_NARROW,
};
pub(crate) use expectation::ResolvedExpectation;
#[cfg(test)]
pub(crate) use expectation::ResolvedExpectationKind;
pub(crate) use line_break::{contains_line_break, escape_inline_text};
pub(crate) use options::{AskCommandArgs, CheckCommandArgs, CheckOptions, RawCheckOptions};
pub(crate) use record::{CheckRecord, CheckRecordOutcome};
pub(crate) use run_report::{
    check_run_error, for_each_unique_report_record, CachedPassRecord, CheckRunError,
    CheckRunReport, InterrogationAnswer, InterrogationAnswerData, InterrogationResult,
    InterrogationTurn, QueryResult,
};
