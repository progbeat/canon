use super::answer::CheckResult;
use crate::config_types::{AgentConfig, ExpectationTarget};

#[derive(Debug, Clone)]
pub(crate) struct SelectedExpectation {
    pub(crate) number: usize,
    pub(crate) id: String,
    pub(crate) display_id: String,
    pub(crate) question: String,
    pub(crate) expected_answer: String,
    pub(crate) instructions: String,
    pub(crate) diff_from: String,
    pub(crate) target: Option<ExpectationTarget>,
    #[allow(dead_code)]
    pub(crate) question_answer_only: bool,
    pub(crate) agent: AgentConfig,
    pub(crate) cooldown: Option<Cooldown>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cooldown {
    pub(crate) pass_seconds: Option<u64>,
    pub(crate) fail_seconds: Option<u64>,
}

impl Cooldown {
    pub(crate) fn duration_for(self, result: CheckResult) -> Option<u64> {
        match result {
            CheckResult::Pass => self.pass_seconds,
            CheckResult::Fail => self.fail_seconds,
        }
    }
}
