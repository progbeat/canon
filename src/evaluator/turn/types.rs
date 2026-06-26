use crate::check::ParsedAnswer;
use crate::token_usage_types::TokenUsage;
use serde::Serialize;

pub(crate) struct EvaluatorTurnContext<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
}

pub(crate) struct ThreadLifecycleLog {
    pub(crate) event: &'static str,
    pub(crate) session_id: String,
    pub(crate) base_instructions: String,
    pub(crate) developer_instructions: String,
    pub(crate) reuse_context: ThreadReuseLogContext,
}

#[derive(Clone, Serialize)]
pub(crate) struct ThreadReuseLogContext {
    #[serde(rename = "visibleTreeOid")]
    pub(crate) visible_tree_oid: String,
    #[serde(rename = "diffBaseTreeOid")]
    pub(crate) diff_base_tree_oid: String,
    #[serde(rename = "checkedTreeOid")]
    pub(crate) checked_tree_oid: String,
    #[serde(rename = "turnPrompt")]
    pub(crate) turn_prompt: String,
    #[serde(rename = "questionContext")]
    pub(crate) question_context: String,
    pub(crate) plugins: Vec<String>,
    pub(crate) ignore: Vec<String>,
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
