use super::answer::CheckResult;
use crate::config_types::{AgentConfig, ExpectationTarget};

#[derive(Debug, Clone)]
pub(crate) struct ResolvedExpectation {
    pub(crate) number: usize,
    pub(crate) id: String,
    pub(crate) display_id: String,
    pub(crate) question: String,
    pub(crate) expected_answer: String,
    pub(crate) question_context: String,
    // Literal `diff-from` config selection. The interrogation session resolves
    // it to the prompt diff tree with the active runtime and last-pass state.
    pub(crate) diff_from: String,
    pub(crate) target: Option<ExpectationTarget>,
    #[allow(dead_code)]
    pub(crate) question_answer_only: bool,
    pub(crate) agent: AgentConfig,
    pub(crate) cooldown: Option<Cooldown>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cooldown {
    // Cached-result cooldowns are status-specific: a compact config populates
    // only `pass_seconds`, while mapping config may populate pass and/or fail.
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
