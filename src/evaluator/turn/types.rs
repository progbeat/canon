use crate::check::ParsedAnswer;
use crate::token_usage_types::TokenUsage;

pub(crate) struct EvaluatorTurnContext<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
}

pub(crate) struct ThreadLifecycleLog {
    pub(crate) event: &'static str,
    pub(crate) session_id: String,
    pub(crate) developer_instructions: String,
}

pub(crate) struct ParsedTurnResponse {
    pub(crate) answer: ParsedAnswer,
    pub(crate) usage: Option<TokenUsage>,
    pub(crate) context_compacted: bool,
}

pub(super) struct RawTurnResponse {
    pub(super) text: String,
    pub(super) usage: Option<TokenUsage>,
    pub(super) context_compacted: bool,
}
