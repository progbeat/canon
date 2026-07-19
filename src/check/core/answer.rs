use serde::{Deserialize, Serialize};

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

    #[test] // xpec: k4
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
