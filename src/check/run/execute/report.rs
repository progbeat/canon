use crate::check::core::types::{
    CachedExpectation, CheckRecord, CheckRunReport, NarrowingStats, SelectedExpectation,
};
use std::collections::BTreeSet;

pub(super) fn check_run_report(
    records: Vec<CheckRecord>,
    non_selected: Vec<SelectedExpectation>,
    cached: Vec<CachedExpectation>,
    counts: CheckRunReportCounts,
    narrowing: NarrowingStats,
) -> CheckRunReport {
    CheckRunReport {
        records,
        non_selected,
        cached,
        evaluated: counts.evaluated,
        selected: counts.selected,
        skipped: counts.skipped,
        silent: counts.silent,
        narrowing,
    }
}

pub(super) struct CheckRunReportCounts {
    pub(super) evaluated: usize,
    pub(super) selected: usize,
    pub(super) skipped: usize,
    pub(super) silent: usize,
}

pub(super) fn skipped_count(
    total_expectations: usize,
    records: &[CheckRecord],
    cached: &[CachedExpectation],
) -> usize {
    total_expectations.saturating_sub(summary_result_count(records, cached))
}

fn summary_result_count(records: &[CheckRecord], cached: &[CachedExpectation]) -> usize {
    let mut seen = BTreeSet::new();
    let mut count = 0usize;
    for record in records {
        if seen.insert(record.id.clone()) {
            count += 1;
        }
    }
    for cached in cached {
        let id = if cached.record.id.is_empty() {
            &cached.expectation.id
        } else {
            &cached.record.id
        };
        if seen.insert(id.clone()) {
            count += 1;
        }
    }
    count
}
