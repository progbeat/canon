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
    evaluator_response_output_schema, matches_answer_pattern, parse_evaluator_response,
    ParsedAnswer, ANSWER_PATTERN, ERROR_INVALID_QUESTION, ERROR_SCOPE_TOO_NARROW,
};
pub(crate) use expectation::{Cooldown, SelectedExpectation};
pub(crate) use line_break::{contains_line_break, is_line_break_char};
pub(crate) use options::{CheckCommandArgs, CheckOptions, RawCheckOptions};
pub(crate) use record::{CheckRecord, CheckRecordOutcome};
pub(crate) use run_report::{
    check_run_error, for_each_unique_report_record, CachedExpectation, CheckRunError,
    CheckRunReport, InterrogationResult, QueryExpectationRecord, QueryResult,
};
