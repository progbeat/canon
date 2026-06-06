pub(super) mod errors;
pub(super) mod types;

#[cfg(test)]
pub(crate) use types::Cooldown;
pub(crate) use types::{
    CheckRecord, CheckRecordOutcome, CheckResult, EvaluatorResponseJson, ParsedAnswer,
    SelectedExpectation, ERROR_UNPARSABLE,
};
