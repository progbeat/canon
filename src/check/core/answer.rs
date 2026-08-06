use serde::{Deserialize, Serialize};

use super::evaluator_response::ERROR_SCOPE_TOO_NARROW;

pub(crate) const RESULT_PASS: &str = "pass";
pub(crate) const RESULT_FAIL: &str = "fail";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CheckResult {
    Pass,
    Fail,
}

impl CheckResult {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            CheckResult::Pass => RESULT_PASS,
            CheckResult::Fail => RESULT_FAIL,
        }
    }

    pub(crate) fn from_expected_answer(expected: &str, observed: &str) -> CheckResult {
        if observed == expected {
            CheckResult::Pass
        } else {
            CheckResult::Fail
        }
    }

    // [4k] Evaluation status accepts only decision fields, never evidence.
    pub(crate) fn from_evaluation(
        expected: &str,
        observed: &str,
        error: Option<&str>,
    ) -> CheckResult {
        if error.is_some() {
            CheckResult::Fail
        } else {
            CheckResult::from_expected_answer(expected, observed)
        }
    }
}

pub(crate) fn assert_evaluation_postconditions(result: CheckResult, error: Option<&str>) {
    // xpec: Eg,l
    assert!(
        matches!(result, CheckResult::Pass | CheckResult::Fail),
        "an xpec must finish as PASS or FAIL"
    );
    // xpec: Eg,l
    assert!(
        error.is_none() || result == CheckResult::Fail,
        "an xpec response error must produce FAIL"
    );
    // ScopeTooNarrow is an internal retry-policy response, never a final
    // evaluator result.
    // xpec: RC,l
    assert_ne!(
        error,
        Some(ERROR_SCOPE_TOO_NARROW),
        "user-visible final evaluator results must not expose ScopeTooNarrow"
    );
}

pub(crate) fn evaluate_final_response(
    expected: &str,
    observed: &str,
    error: Option<&str>,
) -> CheckResult {
    let result = CheckResult::from_evaluation(expected, observed, error);
    assert_evaluation_postconditions(result, error);
    result
}

pub(super) fn default_check_result() -> CheckResult {
    CheckResult::Fail
}

impl std::fmt::Display for CheckResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::CheckResult;

    #[test] // xpec: Eg
    fn observed_answer_must_match_expected_answer_exactly() {
        assert_eq!(
            CheckResult::from_expected_answer("yes", "yes"),
            CheckResult::Pass
        );
        assert_eq!(
            CheckResult::from_expected_answer("no", "No — not found"),
            CheckResult::Fail
        );
        assert_eq!(
            CheckResult::from_expected_answer("yes", "YES"),
            CheckResult::Fail
        );
    }
}
