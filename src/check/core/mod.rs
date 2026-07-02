mod answer;
pub(super) mod errors;
mod evaluator_response;
mod expectation;
mod line_break;
mod options;
mod record;
mod run_report;

pub(crate) use answer::CheckResult;
pub(crate) use errors::INTERNAL_ERROR_UNPARSABLE;
pub(crate) use evaluator_response::{
    evaluator_response_output_schema_for_scope, matches_answer_pattern,
    parse_evaluator_response_for_short_id, EvaluatorResponseParseError,
    EvaluatorResponseSchemaScope, ParsedAnswer, ANSWER_PATTERN, ERROR_INVALID_QUESTION,
    ERROR_SCOPE_TOO_NARROW,
};
pub(crate) use expectation::{Cooldown, SelectedExpectation};
pub(crate) use line_break::{contains_line_break, is_line_break_char};
pub(crate) use options::{AskCommandArgs, CheckCommandArgs, CheckOptions, RawCheckOptions};
pub(crate) use record::{CheckRecord, CheckRecordOutcome};
pub(crate) use run_report::{
    check_run_error, for_each_unique_report_record, interrupted_check_run_error, BlockedCheckHook,
    CachedExpectation, CheckRunError, CheckRunReport, InterrogationAnswer, InterrogationResult,
    QueryResult,
};
