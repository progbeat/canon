use crate::check::core::{
    for_each_unique_report_record, CachedExpectation, CheckRecord, CheckRunReport,
};

pub(super) fn check_run_report(
    records: Vec<CheckRecord>,
    cached: Vec<CachedExpectation>,
    counts: CheckRunReportCounts,
) -> CheckRunReport {
    CheckRunReport {
        records,
        cached,
        blocked_hooks: Vec::new(),
        skipped: counts.skipped,
    }
}

pub(super) struct CheckRunReportCounts {
    pub(super) skipped: usize,
}

pub(crate) fn skipped_count(
    total_expectations: usize,
    records: &[CheckRecord],
    cached: &[CachedExpectation],
) -> usize {
    let mut unique_records = 0usize;
    for_each_unique_report_record(records, cached, |_| unique_records += 1);
    total_expectations.saturating_sub(unique_records)
}
