use crate::config_types::AgentConfig;
use crate::history::history_cache_key;
use crate::time::{format_record_timestamp, unix_timestamp};
use crate::token_usage_types::TokenUsage;
use serde::{de, Deserialize, Serialize};
use serde_json::Value;
use std::ffi::OsString;
use std::path::PathBuf;

// Shared check data types and answer-state classification.

pub(crate) const RESULT_PASS: &str = "pass";
pub(crate) const RESULT_FAIL: &str = "fail";
pub(crate) const ERROR_INSUFFICIENT_EVIDENCE: &str = "insufficient-evidence";
pub(crate) const ERROR_INVALID_QUESTION: &str = "invalid-question";
pub(crate) const ERROR_UNPARSABLE: &str = "unparsable";

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

    pub(crate) fn cache_key(self) -> String {
        format!(
            "pass={};fail={}",
            cooldown_key_part(self.pass_seconds),
            cooldown_key_part(self.fail_seconds)
        )
    }
}

fn cooldown_key_part(seconds: Option<u64>) -> String {
    seconds
        .map(|seconds| seconds.to_string())
        .unwrap_or_else(|| "none".to_string())
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
        ParsedAnswer::error_with_q_scope_suggestion(error, evidence, None)
    }

    pub(crate) fn error_with_q_scope_suggestion(
        error: String,
        evidence: String,
        q_scope_suggestion: Option<Vec<String>>,
    ) -> ParsedAnswer {
        ParsedAnswer {
            answer: error.clone(),
            error: Some(error),
            evidence,
            scope: Vec::new(),
            q_scope_suggestion,
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
    pub(crate) fn from_error(error: Option<&str>) -> ObservedAnswerState {
        match error {
            None => ObservedAnswerState::Answer,
            Some(ERROR_INSUFFICIENT_EVIDENCE) => ObservedAnswerState::InsufficientEvidence,
            Some(ERROR_INVALID_QUESTION) => ObservedAnswerState::InvalidQuestion,
            Some(ERROR_UNPARSABLE) => ObservedAnswerState::Unparsable,
            Some(_) => ObservedAnswerState::Unknown,
        }
    }

    pub(crate) fn requires_human_review(self) -> bool {
        !matches!(self, ObservedAnswerState::Answer)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluatorResponseJson {
    #[serde(default, deserialize_with = "deserialize_optional_answer")]
    pub(crate) answer: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_error")]
    pub(crate) error: Option<String>,
    pub(crate) evidence: String,
    #[serde(
        rename = "qScopeSuggestion",
        deserialize_with = "deserialize_q_scope_suggestion"
    )]
    pub(crate) q_scope_suggestion: Vec<String>,
}

impl EvaluatorResponseJson {
    pub(crate) fn validate_schema(&self) -> Result<(), String> {
        let has_answer = self.answer.is_some();
        let has_error = self.error.is_some();
        if has_answer == has_error {
            return Err(
                "evaluator response must contain exactly one of answer or error".to_string(),
            );
        }
        if let Some(answer) = self.answer.as_deref() {
            if answer.is_empty() || contains_schema_single_line_violation(answer) {
                return Err("answer must be a non-empty single-line string".to_string());
            }
        }
        if let Some(error) = self.error.as_deref() {
            if !matches!(
                error,
                ERROR_INSUFFICIENT_EVIDENCE | ERROR_INVALID_QUESTION | ERROR_UNPARSABLE
            ) {
                return Err(format!("unsupported evaluator error: {}", error));
            }
        }
        // Interrogation Policy keeps `qScopeSuggestion` schema validation to
        // required non-empty single-line strings. Repository-relative scope
        // syntax is not part of response-schema validity; syntax and semantic
        // sufficiency are later narrowing policy checks, which accept a claim
        // only after an independent answer-producing turn.
        // Interrogation Policy's JSON Schema sets `minItems: 1` for
        // `qScopeSuggestion`, so an empty array is a response-schema error.
        if self.q_scope_suggestion.is_empty() {
            return Err("qScopeSuggestion must contain at least one path".to_string());
        }
        // Each item follows the schema's `minLength: 1` and
        // `pattern: "^[^\\r\\n]*$"` constraints.
        for item in &self.q_scope_suggestion {
            if item.is_empty() || contains_schema_single_line_violation(item) {
                return Err(
                    "qScopeSuggestion items must be non-empty single-line strings".to_string(),
                );
            }
        }
        Ok(())
    }
}

fn contains_schema_single_line_violation(value: &str) -> bool {
    value.chars().any(|char| matches!(char, '\r' | '\n'))
}

fn deserialize_optional_answer<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: de::Deserializer<'de>,
{
    deserialize_optional_string_field(deserializer, "answer")
}

fn deserialize_optional_error<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: de::Deserializer<'de>,
{
    deserialize_optional_string_field(deserializer, "error")
}

fn deserialize_optional_string_field<'de, D>(
    deserializer: D,
    field_name: &'static str,
) -> Result<Option<String>, D::Error>
where
    D: de::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_null() {
        return Err(de::Error::custom(format!(
            "{} must not be null",
            field_name
        )));
    }
    String::deserialize(value)
        .map(Some)
        .map_err(de::Error::custom)
}

fn deserialize_q_scope_suggestion<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: de::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_null() {
        return Err(de::Error::custom("qScopeSuggestion must not be null"));
    }
    Vec::<String>::deserialize(value).map_err(de::Error::custom)
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
// Deserialization keeps result/prompt/expected metadata optional so
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
        let error = self.error.as_deref()?;
        if ObservedAnswerState::from_error(Some(error)).requires_human_review() {
            Some(error)
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
    // `--keep-going` continues after non-pass results among selected
    // expectations; it does not bypass default cache-based selection.
    pub(crate) keep_going: bool,
    pub(crate) ignore_cache: bool,
    pub(crate) ignore_cooldown: bool,
    pub(crate) break_after_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RawCheckOptions {
    pub(crate) keep_going: bool,
    pub(crate) ignore_cache: bool,
    pub(crate) ignore_cooldown: bool,
    pub(crate) break_after_tokens: Option<u64>,
    pub(crate) selectors: Vec<OsString>,
}

impl RawCheckOptions {
    pub(crate) fn is_empty(&self) -> bool {
        !self.keep_going
            && !self.ignore_cache
            && !self.ignore_cooldown
            && self.break_after_tokens.is_none()
            && self.selectors.is_empty()
    }
}

pub(crate) struct CheckCommandArgs {
    pub(crate) config_path: PathBuf,
    pub(crate) tree: String,
    pub(crate) against_tree: String,
    pub(crate) against_tree_explicit: bool,
    pub(crate) no_sandbox: bool,
    pub(crate) query: Option<String>,
    pub(crate) query_preset: Option<String>,
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
