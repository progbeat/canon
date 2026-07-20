use crate::config_types::{AgentConfig, Cooldown, ExpectationTarget, ExpectationTo};

#[derive(Debug, Clone)]
pub(crate) struct ResolvedExpectation {
    pub(crate) number: usize,
    pub(crate) id: String,
    pub(crate) display_id: String,
    pub(crate) to: ExpectationTo,
    // [IJ] Final canon check order uses this ascending before latest fail time.
    pub(crate) rank: i64,
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
