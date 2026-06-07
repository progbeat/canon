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

    // Used by evaluator turns, history loading, and selection filters whenever
    // an observed answer must be classified against the current expectation.
    pub(crate) fn from_expected_answer(expected: &str, observed: &str) -> CheckResult {
        if observed == expected || observed_yes_no_answer(observed) == Some(expected) {
            CheckResult::Pass
        } else {
            CheckResult::Fail
        }
    }
}

fn observed_yes_no_answer(observed: &str) -> Option<&'static str> {
    let observed = observed.trim_start();
    let (answer, rest) = observed.split_once(char::is_whitespace)?;
    let had_answer_separator = answer.ends_with([':', '-', '.', ',', ';', '\u{2013}', '\u{2014}']);
    let answer = answer.trim_end_matches([':', '-', '.', ',', ';', '\u{2013}', '\u{2014}']);
    let canonical = if answer.eq_ignore_ascii_case("yes") {
        "yes"
    } else if answer.eq_ignore_ascii_case("no") {
        "no"
    } else {
        return None;
    };
    let rest = rest.trim_start();
    if rest.is_empty()
        || (!had_answer_separator
            && !rest.starts_with([':', '-', '.', ',', ';', '\u{2013}', '\u{2014}']))
    {
        return None;
    }
    Some(canonical)
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
    use super::{observed_yes_no_answer, CheckResult};

    #[test]
    fn yes_no_answers_with_explanatory_separator_match_expected_answer() {
        assert_eq!(observed_yes_no_answer("No — not found"), Some("no"));
        assert_eq!(observed_yes_no_answer("YES: supported"), Some("yes"));
        assert_eq!(
            CheckResult::from_expected_answer("no", "No — not found"),
            CheckResult::Pass
        );
    }

    #[test]
    fn exact_answers_without_yes_no_separator_still_compare_exactly() {
        assert_eq!(observed_yes_no_answer("No evidence"), None);
        assert_eq!(
            CheckResult::from_expected_answer("no", "No evidence"),
            CheckResult::Fail
        );
        assert_eq!(
            CheckResult::from_expected_answer("maybe", "maybe — plausible"),
            CheckResult::Fail
        );
    }
}
