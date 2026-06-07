pub(super) mod errors;
pub(super) mod types;

pub(crate) use types::{
    CheckRecord, CheckRecordOutcome, CheckResult, EvaluatorResponseJson, ParsedAnswer,
    SelectedExpectation, ERROR_UNPARSABLE,
};
