use crate::check::ParsedAnswer;
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
    #[serde(rename = "inPlace")]
    pub(crate) in_place: bool,
    #[serde(rename = "visibleTreeOid", skip_serializing_if = "Option::is_none")]
    pub(crate) visible_tree_oid: Option<String>,
    #[serde(rename = "diffBaseTreeOid", skip_serializing_if = "Option::is_none")]
    pub(crate) diff_base_tree_oid: Option<String>,
    #[serde(rename = "checkedTreeOid", skip_serializing_if = "Option::is_none")]
    pub(crate) checked_tree_oid: Option<String>,
    #[serde(rename = "turnPrompt")]
    pub(crate) turn_prompt: String,
    #[serde(rename = "questionContext")]
    pub(crate) question_context: String,
    pub(crate) plugins: Vec<String>,
    pub(crate) ignore: Vec<String>,
}

pub(crate) struct ParsedTurnResponse {
    pub(crate) answer: ParsedAnswer,
    pub(crate) context_compacted: bool,
    pub(crate) schema_valid: bool,
}

pub(super) struct RawTurnResponse {
    pub(super) text: String,
    pub(super) context_compacted: bool,
}
