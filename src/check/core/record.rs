use super::answer::{default_check_result, CheckResult};
use super::expectation::ResolvedExpectation;
use crate::config_types::ExpectationTo;
use crate::time::{format_record_timestamp, unix_timestamp};
use serde::Deserialize;

// In-memory check record used by cache reuse, runtime logs, gate diagnostics,
// and check output. Runtime records created from evaluator responses receive a
// repository-native `visibleTreeOid` before they reach last-result storage;
// in-place records have no tree identity.
// The type deliberately does not implement `Serialize`; persisted history and
// runtime-log records must go through dedicated render structs, which write the
// full expectation ID and never the human display/selector prefix.
// Deserialization keeps result/question/expected-answer metadata optional so
// compact persisted or diagnostic records do not get confused with real empty
// strings.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CheckRecord {
    pub(crate) timestamp: String,
    #[serde(default = "default_check_result")]
    pub(crate) result: CheckResult,
    #[serde(default, skip)]
    pub(crate) to: ExpectationTo,
    #[serde(default)]
    #[serde(alias = "prompt")]
    pub(crate) question: Option<String>,
    #[serde(default)]
    #[serde(alias = "expected")]
    pub(crate) expected_answer: Option<String>,
    pub(crate) observed: String,
    #[serde(default)]
    pub(crate) error: Option<String>,
    pub(crate) evidence: Option<String>,
    #[serde(rename = "qScope", alias = "scope")]
    pub(crate) scope: Vec<String>,
    #[serde(default, rename = "qScopeSuggestion", alias = "suggestedQScope")]
    pub(crate) q_scope_suggestion: Option<Vec<String>>,
    #[serde(
        default,
        rename = "visibleTreeOid",
        alias = "scopeTreeOid",
        alias = "scopeHash"
    )]
    pub(crate) visible_tree_oid: Option<String>,
    // Git-backed evaluator turns attach the resolved diff base used for the
    // prompt-rendered diff so wrong-answer stdout can print the public
    // `diff-from:` line. Persistent state stores the full OID; stdout uses the
    // Git-produced abbreviation carried only in memory.
    #[serde(default, rename = "diffFrom")]
    pub(crate) diff_from: Option<String>,
    #[serde(default, rename = "diffFromTreeOid")]
    pub(crate) diff_from_tree_oid: Option<String>,
    #[serde(default, skip)]
    pub(crate) diff_from_tree_oid_abbrev: Option<String>,
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default, skip)]
    pub(crate) display_id: String,
}

pub(crate) struct CheckRecordOutcome {
    pub(crate) result: CheckResult,
    pub(crate) observed: String,
    pub(crate) error: Option<String>,
    pub(crate) evidence: Option<String>,
    pub(crate) scope: Vec<String>,
    // Evaluator-response feedback used by q-scope verification and optional
    // stdout hints. A persisted final response retains this unverified proposal;
    // its separately stored `qScope` is the scope actually used for that response.
    pub(crate) q_scope_suggestion: Option<Vec<String>>,
    pub(crate) visible_tree_oid: Option<String>,
    pub(crate) diff_from: Option<String>,
    pub(crate) diff_from_tree_oid: Option<String>,
    pub(crate) diff_from_tree_oid_abbrev: Option<String>,
}

impl CheckRecordOutcome {
    pub(crate) fn new(
        result: CheckResult,
        observed: String,
        error: Option<String>,
        evidence: Option<String>,
        scope: Vec<String>,
    ) -> CheckRecordOutcome {
        CheckRecordOutcome {
            result,
            observed,
            error,
            evidence,
            scope,
            q_scope_suggestion: None,
            visible_tree_oid: None,
            diff_from: None,
            diff_from_tree_oid: None,
            diff_from_tree_oid_abbrev: None,
        }
    }
}

impl CheckRecord {
    // Output, logs, gate, lazy reset, and run control all use these accessors
    // instead of duplicating optional-history-record semantics at each call
    // site.
    pub(crate) fn passed(&self) -> bool {
        self.result == CheckResult::Pass
    }

    pub(crate) fn requires_human_review(&self) -> bool {
        self.human_review_reason().is_some()
    }

    pub(crate) fn human_review_reason(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn current_from_expectation(
        expectation: &ResolvedExpectation,
        outcome: CheckRecordOutcome,
    ) -> Result<CheckRecord, String> {
        let expected_answer = expectation.expected_answer().to_string();
        Ok(CheckRecord {
            timestamp: format_record_timestamp(unix_timestamp()?),
            id: expectation.require_configured_id()?.to_string(),
            display_id: expectation.display_id.clone(),
            result: outcome.result,
            to: expectation.to,
            question: Some(expectation.question.clone()),
            expected_answer: Some(expected_answer),
            observed: outcome.observed,
            error: outcome.error,
            evidence: outcome.evidence,
            scope: outcome.scope,
            q_scope_suggestion: outcome.q_scope_suggestion,
            visible_tree_oid: outcome.visible_tree_oid,
            diff_from: outcome.diff_from,
            diff_from_tree_oid: outcome.diff_from_tree_oid,
            diff_from_tree_oid_abbrev: outcome.diff_from_tree_oid_abbrev,
        })
    }

    pub(crate) fn question_text(&self) -> &str {
        self.question.as_deref().unwrap_or("")
    }

    pub(crate) fn expected_answer_text(&self) -> Option<&str> {
        self.expected_answer.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::CheckRecord;
    use serde_json::json;

    #[test] // xpec: gN
    fn deserialized_record_scope_has_q_scope_semantics() {
        let record: CheckRecord = serde_json::from_value(json!({
            "timestamp": "2026-01-01T00:00:00Z",
            "observed": "yes",
            "evidence": "evidence",
            "qScope": ["src"]
        }))
        .unwrap();

        assert_eq!(record.scope, ["src"]);
        assert!(serde_json::from_value::<CheckRecord>(json!({
            "timestamp": "2026-01-01T00:00:00Z",
            "observed": "yes",
            "evidence": "evidence",
            "visibleScope": ["src"]
        }))
        .is_err());
    }
}
