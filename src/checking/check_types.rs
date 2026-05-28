use crate::config_types::AgentConfig;
use crate::history_cache_key::history_cache_key;
use crate::time::{format_record_timestamp, unix_timestamp};
use crate::token_usage_types::TokenUsage;
use crate::{
    ERROR_INSUFFICIENT_EVIDENCE, ERROR_INVALID_QUESTION, ERROR_UNPARSABLE, RESULT_FAIL, RESULT_PASS,
};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::PathBuf;

// Shared check data types and answer-state classification.

pub(crate) fn contains_line_break(value: &str) -> bool {
    value.chars().any(is_line_break_char)
}

pub(crate) fn is_line_break_char(char: char) -> bool {
    matches!(
        char,
        '\n' | '\r' | '\u{000b}' | '\u{000c}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
    )
}

#[derive(Debug, Clone)]
pub(crate) struct SelectedExpectation {
    pub(crate) number: usize,
    pub(crate) id: String,
    pub(crate) display_id: String,
    pub(crate) q: String,
    pub(crate) a: String,
    #[allow(dead_code)]
    pub(crate) prompt_scope: Vec<String>,
    pub(crate) agent: AgentConfig,
    pub(crate) cooldown: Option<Cooldown>,
    #[allow(dead_code)]
    pub(crate) thinking: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cooldown {
    pub(crate) seconds: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedAnswer {
    pub(crate) answer: String,
    pub(crate) error: Option<String>,
    pub(crate) evidence: String,
    pub(crate) scope: Vec<String>,
    pub(crate) q_scope_suggestion: Option<Vec<String>>,
}

impl ParsedAnswer {
    pub(crate) fn answer(
        answer: String,
        evidence: String,
        q_scope_suggestion: Option<Vec<String>>,
    ) -> ParsedAnswer {
        ParsedAnswer {
            answer,
            error: None,
            evidence,
            scope: Vec::new(),
            q_scope_suggestion,
        }
    }

    pub(crate) fn error(error: String, evidence: String) -> ParsedAnswer {
        ParsedAnswer {
            answer: error.clone(),
            error: Some(error),
            evidence,
            scope: Vec::new(),
            q_scope_suggestion: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObservedAnswerState {
    Answer,
    InsufficientEvidence,
    InvalidQuestion,
    Unparsable,
    Unknown,
}

impl ObservedAnswerState {
    pub(crate) fn from_observed(observed: &str) -> ObservedAnswerState {
        match observed {
            ERROR_INSUFFICIENT_EVIDENCE => ObservedAnswerState::InsufficientEvidence,
            ERROR_INVALID_QUESTION => ObservedAnswerState::InvalidQuestion,
            ERROR_UNPARSABLE => ObservedAnswerState::Unparsable,
            _ if contains_line_break(observed) => ObservedAnswerState::Unknown,
            _ => ObservedAnswerState::Answer,
        }
    }

    pub(crate) fn from_expected_and_observed(
        _expected: &str,
        observed: &str,
    ) -> ObservedAnswerState {
        ObservedAnswerState::from_observed(observed)
    }

    pub(crate) fn requires_human_review(self) -> bool {
        !matches!(self, ObservedAnswerState::Answer)
    }

    pub(crate) fn is_reusable_history(self) -> bool {
        matches!(self, ObservedAnswerState::Answer)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluatorResponseJson {
    #[serde(default)]
    pub(crate) answer: Option<String>,
    #[serde(default)]
    pub(crate) error: Option<String>,
    pub(crate) evidence: String,
    #[serde(default, rename = "qScopeSuggestion")]
    pub(crate) q_scope_suggestion: Option<Vec<String>>,
}

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

    pub(crate) fn from_expected_answer(expected: &str, observed: &str) -> CheckResult {
        if observed == expected {
            CheckResult::Pass
        } else {
            CheckResult::Fail
        }
    }
}

fn default_check_result() -> CheckResult {
    CheckResult::Fail
}

impl std::fmt::Display for CheckResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

// In-memory check record used by history reuse, runtime logs, gate diagnostics,
// and check output. Runtime records created from evaluator responses receive a
// repository-native `visibleTreeOid` before they reach answer-history append.
// The type deliberately does not implement `Serialize`; persisted history and
// runtime-log records must go through dedicated render structs, which write the
// full expectation ID and never the human display/selector prefix.
// Deserialization keeps result/prompt/expected metadata optional so older or
// spec-minimal history records that contain only the cache-required prefix do
// not get confused with real empty strings. Cache readers recompute current
// result from observed vs the current expected answer.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CheckRecord {
    pub(crate) timestamp: String,
    #[serde(default)]
    pub(crate) number: usize,
    #[serde(default = "default_check_result")]
    pub(crate) result: CheckResult,
    #[serde(default)]
    pub(crate) prompt: Option<String>,
    #[serde(default)]
    pub(crate) expected: Option<String>,
    pub(crate) observed: String,
    #[serde(default)]
    pub(crate) error: Option<String>,
    pub(crate) evidence: String,
    #[serde(rename = "qScope", alias = "scope")]
    pub(crate) scope: Vec<String>,
    #[serde(default, rename = "suggestedQScope")]
    pub(crate) suggested_q_scope: Option<Vec<String>>,
    #[serde(rename = "visibleTreeOid", alias = "scopeTreeOid", alias = "scopeHash")]
    pub(crate) visible_tree_oid: String,
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default, skip)]
    pub(crate) display_id: String,
    #[allow(dead_code)]
    #[serde(default, rename = "cacheKey")]
    pub(crate) cache_key: Option<String>,
}

pub(crate) struct CheckRecordOutcome {
    pub(crate) result: CheckResult,
    pub(crate) observed: String,
    pub(crate) error: Option<String>,
    pub(crate) evidence: String,
    pub(crate) scope: Vec<String>,
    pub(crate) suggested_q_scope: Option<Vec<String>>,
    pub(crate) visible_tree_oid: String,
}

impl CheckRecord {
    pub(crate) fn passed(&self) -> bool {
        self.result == CheckResult::Pass
    }

    pub(crate) fn review_error_text(&self) -> Option<&str> {
        if let Some(error) = self.error.as_deref() {
            return Some(error);
        }
        let expected = self.expected_text()?;
        if ObservedAnswerState::from_expected_and_observed(expected, &self.observed)
            .requires_human_review()
        {
            Some(&self.observed)
        } else {
            None
        }
    }

    pub(crate) fn current_from_expectation(
        agent: &AgentConfig,
        expectation: &SelectedExpectation,
        outcome: CheckRecordOutcome,
    ) -> Result<CheckRecord, String> {
        Ok(Self::from_expectation(
            format_record_timestamp(unix_timestamp()?),
            expectation,
            Some(history_cache_key(agent, expectation)),
            outcome,
        ))
    }

    pub(crate) fn from_expectation(
        timestamp: String,
        expectation: &SelectedExpectation,
        cache_key: Option<String>,
        outcome: CheckRecordOutcome,
    ) -> CheckRecord {
        CheckRecord {
            timestamp,
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: outcome.result,
            prompt: Some(expectation.q.clone()),
            expected: Some(expectation.a.clone()),
            observed: outcome.observed,
            error: outcome.error,
            evidence: outcome.evidence,
            scope: outcome.scope,
            suggested_q_scope: outcome.suggested_q_scope,
            visible_tree_oid: outcome.visible_tree_oid,
            cache_key,
        }
    }

    pub(crate) fn prompt_text(&self) -> &str {
        self.prompt.as_deref().unwrap_or("")
    }

    pub(crate) fn expected_text(&self) -> Option<&str> {
        self.expected.as_deref()
    }
}

pub(crate) struct CheckOptions {
    // CLI-expanded selected expectations before check-only work-saving filters.
    pub(crate) selected: Vec<SelectedExpectation>,
    pub(crate) non_selected: Vec<SelectedExpectation>,
    pub(crate) selectors_provided: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    // Command-selector misses before check-only work-saving filters.
    pub(crate) skipped: usize,
    // `--all` treats the collected expectations as an explicit fresh
    // evaluation request and continues after non-pass results.
    pub(crate) check_all: bool,
    pub(crate) ignore_cache: bool,
    pub(crate) ignore_cooldown: bool,
    pub(crate) break_after_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RawCheckOptions {
    pub(crate) check_all: bool,
    pub(crate) ignore_cache: bool,
    pub(crate) ignore_cooldown: bool,
    pub(crate) break_after_tokens: Option<u64>,
    pub(crate) selectors: Vec<OsString>,
}

impl RawCheckOptions {
    pub(crate) fn is_empty(&self) -> bool {
        !self.check_all
            && !self.ignore_cache
            && !self.ignore_cooldown
            && self.break_after_tokens.is_none()
            && self.selectors.is_empty()
    }
}

pub(crate) struct CheckCommandArgs {
    pub(crate) config_path: PathBuf,
    pub(crate) query: Option<String>,
    pub(crate) query_scope: Vec<String>,
    pub(crate) options: RawCheckOptions,
}

pub(crate) struct InterrogationResult {
    pub(crate) record: CheckRecord,
    pub(crate) turn_usage: Option<TokenUsage>,
    pub(crate) context_compacted: bool,
    pub(crate) stop_after_current_expectation: bool,
}

#[derive(Debug)]
pub(crate) struct QueryResult {
    pub(crate) answer: ParsedAnswer,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct NarrowingStats {
    pub(crate) attempted: usize,
    pub(crate) accepted: usize,
    pub(crate) rejected: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct CachedExpectation {
    pub(crate) expectation: SelectedExpectation,
    pub(crate) record: CheckRecord,
}

#[derive(Debug, Clone)]
pub(crate) struct CheckRunReport {
    pub(crate) records: Vec<CheckRecord>,
    #[allow(dead_code)]
    pub(crate) non_selected: Vec<SelectedExpectation>,
    pub(crate) cached: Vec<CachedExpectation>,
    // Freshly evaluated expectations in this run.
    pub(crate) evaluated: usize,
    // Expectations selected for evaluator work in this run.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) selected: usize,
    // Expectations not covered by pass, fail, or human-review summary
    // categories.
    pub(crate) skipped: usize,
    // Skipped expectations that were selected by the command but intentionally
    // produce no per-expectation stdout.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) silent: usize,
    // Kept for internal assertions around scope-narrowing behavior; public
    // output and runtime logs rely on the per-event narrowing records instead.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) narrowing: NarrowingStats,
}

#[derive(Debug, Clone)]
pub(crate) struct CheckRunError {
    pub(crate) error: String,
    pub(crate) report: Box<CheckRunReport>,
}

pub(crate) fn check_run_error(error: String, report: CheckRunReport) -> CheckRunError {
    CheckRunError {
        error,
        report: Box::new(report),
    }
}
