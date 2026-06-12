use crate::check::core::{CachedExpectation, CheckRecord, CheckRunReport};
use std::collections::BTreeSet;

pub(super) fn check_run_report(
    records: Vec<CheckRecord>,
    cached: Vec<CachedExpectation>,
    counts: CheckRunReportCounts,
) -> CheckRunReport {
    CheckRunReport {
        records,
        cached,
        evaluated: counts.evaluated,
        skipped: counts.skipped,
    }
}

pub(super) struct CheckRunReportCounts {
    pub(super) evaluated: usize,
    pub(super) skipped: usize,
}

pub(super) fn skipped_count(
    total_expectations: usize,
    records: &[CheckRecord],
    cached: &[CachedExpectation],
) -> usize {
    let mut unique_records = 0usize;
    let mut seen = BTreeSet::new();
    for record in records {
        if seen.insert(record.id.clone()) {
            unique_records += 1;
        }
    }
    for cached in cached {
        if cached.record.requires_human_review() {
            continue;
        }
        let id = if cached.record.id.is_empty() {
            &cached.expectation.id
        } else {
            &cached.record.id
        };
        if seen.insert(id.clone()) {
            unique_records += 1;
        }
    }
    total_expectations.saturating_sub(unique_records)
}
