use crate::check::{CheckRecord, CheckResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LastResultStatus {
    Pass,
    Fail,
}

impl LastResultStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            LastResultStatus::Pass => "pass",
            LastResultStatus::Fail => "fail",
        }
    }

    pub(in crate::xpec_state) fn file_name(self) -> &'static str {
        match self {
            LastResultStatus::Pass => "last-pass.json",
            LastResultStatus::Fail => "last-fail.json",
        }
    }

    pub(super) fn check_result(self) -> CheckResult {
        match self {
            LastResultStatus::Pass => CheckResult::Pass,
            LastResultStatus::Fail => CheckResult::Fail,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LastResult {
    // [g2,Sh] This is deliberate cross-invocation xpec history, not
    // invocation-local execution state. Git-backed evaluator interrogation
    // responses store the prompt-rendered diff base as `diffFrom` and
    // `diffFromTreeOid`; records from paths without such an interrogation leave
    // those fields absent. An in-place status result also omits checkedTreeOid,
    // so a pass there does not define a Git-tree checkpoint. The containing
    // xpec directory is keyed by the full expectation ID; the JSON body does not
    // persist the expectation ID or human display prefix.
    #[serde(rename = "responseTimestamp")]
    pub(crate) response_timestamp: String,
    #[serde(rename = "updatedTimestamp")]
    pub(crate) updated_timestamp: String,
    pub(crate) status: LastResultStatus,
    pub(crate) response: LastResultResponse,
    #[serde(rename = "qScope", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) q_scope: Vec<String>,
    #[serde(
        rename = "visibleScope",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub(crate) visible_scope: Vec<String>,
    #[serde(
        rename = "checkedTreeOid",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) checked_tree_oid: Option<String>,
    #[serde(
        rename = "visibleTreeOid",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) visible_tree_oid: Option<String>,
    #[serde(rename = "diffFrom", default, skip_serializing_if = "Option::is_none")]
    pub(crate) diff_from: Option<String>,
    #[serde(
        rename = "diffFromTreeOid",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) diff_from_tree_oid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LastResultResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) evidence: Option<String>,
    #[serde(
        rename = "qScopeSuggestion",
        default,
        deserialize_with = "deserialize_present_q_scope_suggestion",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) q_scope_suggestion: Option<Vec<String>>,
}

fn deserialize_present_q_scope_suggestion<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Vec::<String>::deserialize(deserializer).map(Some)
}

impl LastResultResponse {
    pub(crate) fn answered(
        answer: impl Into<String>,
        evidence: impl Into<String>,
        q_scope_suggestion: Option<Vec<String>>,
    ) -> Self {
        Self {
            answer: Some(answer.into()),
            error: None,
            evidence: Some(evidence.into()),
            q_scope_suggestion,
        }
    }

    #[cfg(test)]
    pub(crate) fn error(error: impl Into<String>, q_scope_suggestion: Option<Vec<String>>) -> Self {
        Self {
            answer: None,
            error: Some(error.into()),
            evidence: None,
            q_scope_suggestion,
        }
    }

    pub(super) fn from_record(record: &CheckRecord) -> Self {
        let q_scope_suggestion = record.q_scope_suggestion.clone();
        if let Some(error) = record.error.clone() {
            Self {
                answer: None,
                error: Some(error),
                evidence: record.evidence.clone(),
                q_scope_suggestion,
            }
        } else if let Some(evidence) = record.evidence.clone() {
            Self::answered(record.observed.clone(), evidence, q_scope_suggestion)
        } else {
            Self {
                answer: Some(record.observed.clone()),
                error: None,
                evidence: None,
                q_scope_suggestion,
            }
        }
    }
}

impl LastResult {
    pub(crate) fn answer(&self) -> Option<&str> {
        self.response.answer.as_deref()
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.response.error.as_deref()
    }

    pub(super) fn evidence(&self) -> Option<String> {
        self.response.evidence.clone()
    }

    pub(crate) fn q_scope_suggestion(&self) -> Option<Vec<String>> {
        // Last-result `response` is the normalized evaluator response; the
        // applied q-scope is stored separately in `qScope`.
        self.response.q_scope_suggestion.clone()
    }
}
