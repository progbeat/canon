use super::evaluator_response::ParsedAnswer;
use super::expectation::ResolvedExpectation;
use super::record::CheckRecord;
use crate::token_usage_types::TokenUsage;
use std::collections::BTreeSet;

pub(crate) struct InterrogationResult {
    pub(crate) record: CheckRecord,
    pub(crate) turn_usage: Option<TokenUsage>,
    pub(crate) context_compacted: bool,
    pub(crate) stop_after_current_expectation: bool,
    pub(crate) interrupted: bool,
}

pub(crate) struct InterrogationAnswer {
    pub(crate) answer: ParsedAnswer,
    pub(crate) visible_tree_oid: String,
    pub(crate) diff_from: Option<String>,
    pub(crate) diff_from_tree_oid: Option<String>,
    pub(crate) diff_from_tree_oid_abbrev: Option<String>,
    pub(crate) turn_usage: Option<TokenUsage>,
    pub(crate) context_compacted: bool,
    pub(crate) stop_after_current_expectation: bool,
    pub(crate) interrupted: bool,
}

#[derive(Debug)]
pub(crate) struct QueryResult {
    pub(crate) answer: ParsedAnswer,
}

#[derive(Debug, Clone)]
pub(crate) struct CachedExpectation {
    pub(crate) expectation: ResolvedExpectation,
    pub(crate) record: CheckRecord,
}

#[derive(Debug, Clone)]
pub(crate) struct CheckRunReport {
    // Structured result records produced by evaluator work in this run.
    pub(crate) records: Vec<CheckRecord>,
    pub(crate) cached: Vec<CachedExpectation>,
    // Expectations not covered by evaluated records or cached results.
    pub(crate) skipped: usize,
}

pub(crate) fn for_each_unique_report_record(
    records: &[CheckRecord],
    cached: &[CachedExpectation],
    mut visit: impl FnMut(&CheckRecord),
) {
    let mut seen = BTreeSet::new();
    for record in records {
        if seen.insert(record.id.clone()) {
            visit(record);
        }
    }
    for cached in cached {
        let id = if cached.record.id.is_empty() {
            &cached.expectation.id
        } else {
            &cached.record.id
        };
        if seen.insert(id.clone()) {
            visit(&cached.record);
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CheckRunError {
    pub(crate) error: String,
    // Runtime errors carry the complete report available at the error boundary.
    pub(crate) report: Box<CheckRunReport>,
}

// `check::run` uses this constructor to attach the partial report to every
// early-returning runtime error.
pub(crate) fn check_run_error(error: String, report: CheckRunReport) -> CheckRunError {
    CheckRunError {
        error,
        report: Box::new(report),
    }
}
