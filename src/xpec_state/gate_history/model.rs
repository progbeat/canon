use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GateHistory {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_pass: Option<GitBackedPass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_fail: Option<GitBackedFail>,
}

#[derive(Clone, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GitBackedPass {
    pub(crate) response_timestamp: String,
    pub(crate) visible_scope: Vec<String>,
    pub(crate) visible_tree_oid: String,
}

#[derive(Clone, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GitBackedFail {
    pub(crate) checked_tree_oid: String,
}
