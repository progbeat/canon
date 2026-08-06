use super::errors::INTERNAL_ERROR_UNPARSABLE;
use super::evaluator_response::{ParsedAnswer, ERROR_INVALID_QUESTION};
use super::record::CheckRecord;
use std::collections::BTreeSet;

pub(crate) struct InterrogationTurn<T> {
    pub(crate) output: T,
    pub(crate) context_compacted: bool,
    pub(crate) interrupted: bool,
}

impl<T> InterrogationTurn<T> {
    pub(crate) fn new(
        output: T,
        context_compacted: bool,
        interrupted: bool,
    ) -> InterrogationTurn<T> {
        InterrogationTurn {
            output,
            context_compacted,
            interrupted,
        }
    }
}

pub(crate) type InterrogationResult = InterrogationTurn<CheckRecord>;

pub(crate) struct InterrogationAnswerData {
    pub(crate) answer: ParsedAnswer,
    pub(crate) visible_tree_oid: Option<String>,
    pub(crate) diff_from: Option<String>,
    pub(crate) diff_from_tree_oid: Option<String>,
    pub(crate) diff_from_tree_oid_abbrev: Option<String>,
}

pub(crate) type InterrogationAnswer = InterrogationTurn<InterrogationAnswerData>;

#[derive(Debug)]
pub(crate) struct QueryResult {
    pub(crate) answer: ParsedAnswer,
    pub(crate) diff_from: Option<String>,
    pub(crate) diff_from_tree_oid_abbrev: Option<String>,
}

impl QueryResult {
    pub(crate) fn human_review_reason(&self) -> Option<&'static str> {
        match self.answer.error.as_deref() {
            Some(ERROR_INVALID_QUESTION) => Some("invalid question"),
            Some(INTERNAL_ERROR_UNPARSABLE) => Some("unparsable evaluator response"),
            None => None,
            Some(_) => Some("unknown evaluator error"),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CachedPassRecord {
    record: CheckRecord,
}

impl CachedPassRecord {
    pub(crate) fn from_cache_candidate(record: CheckRecord) -> Option<CachedPassRecord> {
        (record.passed() && !record.requires_human_review()).then_some(CachedPassRecord { record })
    }

    pub(crate) fn as_check_record(&self) -> &CheckRecord {
        &self.record
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CheckRunReport {
    // Structured result records produced by evaluator work in this run.
    pub(crate) records: Vec<CheckRecord>,
    // [m,w] Cached Result is pass-only by definition. Keep that invariant in
    // the element type so feedback and failure history cannot observe a
    // representable cached failure.
    pub(crate) cached_passes: Vec<CachedPassRecord>,
    // Expectations not covered by evaluated records or cached results.
    pub(crate) pending: usize,
}

pub(crate) fn for_each_unique_report_record(
    records: &[CheckRecord],
    cached_passes: &[CachedPassRecord],
    mut visit: impl FnMut(&CheckRecord),
) {
    let mut seen = BTreeSet::new();
    for record in records {
        if seen.insert(record.id.clone()) {
            visit(record);
        }
    }
    for cached_pass in cached_passes {
        let record = cached_pass.as_check_record();
        if seen.insert(record.id.clone()) {
            visit(record);
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CheckRunError {
    pub(crate) error: String,
    // Runtime errors carry the complete report available at the error boundary.
    pub(crate) report: Box<CheckRunReport>,
}

// `check::engine` uses this constructor to attach the partial report to every
// early-returning runtime error.
pub(crate) fn check_run_error(error: String, report: CheckRunReport) -> CheckRunError {
    CheckRunError {
        error,
        report: Box::new(report),
    }
}
