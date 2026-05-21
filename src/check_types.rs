use crate::config_types::AgentConfig;
use crate::history_cache_key::history_cache_key;
use crate::time::{format_record_timestamp, unix_timestamp};
use crate::token_usage_types::TokenUsage;
use crate::{
    EMPTY_EVIDENCE_OBSERVED, OBSERVED_IDK, OBSERVED_MALFORMED, RESULT_FAIL, RESULT_PASS,
    UNPARSEABLE_OBSERVED,
};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::PathBuf;

// This module owns shared check data types and answer-state classification only.
// It is not the whole interrogation policy: response key order is enforced in
// evaluator_json.rs, per-turn record finalization is in evaluator_turn.rs,
// thread reuse is in check_interrogation.rs, and retry/narrowing/history
// orchestration is in check.rs and check_interrogation_records.rs. A verdict on
// the full interrogation policy cannot be made from this file in isolation.

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
    pub(crate) cooldown: Option<Cooldown>,
    pub(crate) thinking: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cooldown {
    pub(crate) seconds: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedAnswer {
    pub(crate) answer: String,
    pub(crate) evidence: String,
    pub(crate) scope: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObservedAnswerState {
    Answer,
    Idk,
    Malformed,
    Unparseable,
    EmptyEvidence,
    Unknown,
}

impl ObservedAnswerState {
    pub(crate) fn from_observed(observed: &str) -> ObservedAnswerState {
        match observed {
            OBSERVED_IDK => ObservedAnswerState::Idk,
            OBSERVED_MALFORMED => ObservedAnswerState::Malformed,
            UNPARSEABLE_OBSERVED => ObservedAnswerState::Unparseable,
            EMPTY_EVIDENCE_OBSERVED => ObservedAnswerState::EmptyEvidence,
            _ if contains_line_break(observed) => ObservedAnswerState::Unknown,
            _ => ObservedAnswerState::Answer,
        }
    }

    pub(crate) fn from_expected_and_observed(
        expected: &str,
        observed: &str,
    ) -> ObservedAnswerState {
        let state = ObservedAnswerState::from_observed(observed);
        if state != ObservedAnswerState::Answer {
            return state;
        }
        // Reserved non-answer tokens are classified above before considering the
        // expectation shape, so even free-form exact-string expectations never
        // reuse `idk`, `malformed`, unparseable, or empty-evidence responses as
        // ordinary exact answers. The expectation only decides which remaining
        // single-line answer vocabulary is valid for pass/fail comparison.
        if is_yes_no_token(expected) {
            if is_yes_no_token(observed) {
                ObservedAnswerState::Answer
            } else {
                ObservedAnswerState::Unknown
            }
        } else if is_option_token(expected) {
            if is_option_token(observed) {
                ObservedAnswerState::Answer
            } else {
                ObservedAnswerState::Unknown
            }
        } else {
            ObservedAnswerState::Answer
        }
    }

    pub(crate) fn requires_human_review(self) -> bool {
        !matches!(self, ObservedAnswerState::Answer)
    }

    pub(crate) fn is_reusable_history(self) -> bool {
        matches!(self, ObservedAnswerState::Answer)
    }
}

fn is_yes_no_token(value: &str) -> bool {
    matches!(value, "yes" | "no")
}

fn is_option_token(value: &str) -> bool {
    value.len() == 1 && value.bytes().all(|byte| byte.is_ascii_lowercase())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluatorResponseJson {
    pub(crate) answer: String,
    pub(crate) evidence: String,
    pub(crate) scope: Vec<String>,
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
}

impl std::fmt::Display for CheckResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

// In-memory check record used by history reuse, runtime logs, gate diagnostics,
// and check output. It deliberately does not implement `Serialize`; persisted
// history and runtime-log records must go through the dedicated render structs
// in `logging_render.rs`, which write the full expectation ID and never the
// human display/selector prefix.
// Deserialization keeps prompt/expected metadata optional so older history
// records that contain only the cache-required prefix do not get confused with
// real empty strings.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CheckRecord {
    pub(crate) timestamp: String,
    #[serde(default)]
    pub(crate) number: usize,
    pub(crate) result: CheckResult,
    #[serde(default)]
    pub(crate) prompt: Option<String>,
    #[serde(default)]
    pub(crate) expected: Option<String>,
    pub(crate) observed: String,
    pub(crate) evidence: String,
    pub(crate) scope: Vec<String>,
    #[serde(rename = "scopeTreeOid", alias = "scopeHash")]
    pub(crate) scope_hash: String,
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default, skip)]
    pub(crate) display_id: String,
    #[serde(default, rename = "cacheKey")]
    pub(crate) cache_key: Option<String>,
}

pub(crate) struct CheckRecordOutcome {
    pub(crate) result: CheckResult,
    pub(crate) observed: String,
    pub(crate) evidence: String,
    pub(crate) scope: Vec<String>,
    pub(crate) scope_hash: String,
}

impl CheckRecord {
    pub(crate) fn passed(&self) -> bool {
        self.result == CheckResult::Pass
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
            evidence: outcome.evidence,
            scope: outcome.scope,
            scope_hash: outcome.scope_hash,
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
    // CLI-expanded expectation candidates. This is not the final selected set:
    // cooldown can remove candidates before the check report records its
    // selected/skipped counts.
    pub(crate) selected: Vec<SelectedExpectation>,
    pub(crate) non_selected: Vec<SelectedExpectation>,
    pub(crate) skipped: usize,
    // `canon check` stops after the first final non-pass by default. `--all`
    // keeps running the full already-selected set.
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
pub(crate) struct CheckRunReport {
    pub(crate) records: Vec<CheckRecord>,
    pub(crate) non_selected: Vec<SelectedExpectation>,
    // Freshly evaluated expectations in this run.
    pub(crate) evaluated: usize,
    // Final selected count after every selection rule has run. This excludes
    // command-selector misses and cooldown matches.
    pub(crate) selected: usize,
    // Final non-selected count.
    pub(crate) skipped: usize,
    // Non-selected expectations that intentionally produce no per-expectation
    // stdout, currently cooldown matches.
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
