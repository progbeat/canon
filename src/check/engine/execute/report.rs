use crate::check::core::{
    for_each_unique_report_record, CachedPassRecord, CheckRecord, CheckRunReport,
};

pub(super) fn check_run_report(
    records: Vec<CheckRecord>,
    cached_passes: Vec<CachedPassRecord>,
    counts: CheckRunReportCounts,
) -> CheckRunReport {
    CheckRunReport {
        records,
        cached_passes,
        pending: counts.pending,
    }
}

pub(super) struct CheckRunReportCounts {
    pub(super) pending: usize,
}

pub(crate) fn pending_count(
    total_expectations: usize,
    records: &[CheckRecord],
    cached_passes: &[CachedPassRecord],
) -> usize {
    let mut unique_records = 0usize;
    for_each_unique_report_record(records, cached_passes, |_| unique_records += 1);
    total_expectations.saturating_sub(unique_records)
}

pub(super) fn append_completed_record(
    report: &mut CheckRunReport,
    total_expectations: usize,
    record: &CheckRecord,
) {
    // [2Z,kK] The public result boundary owns progress accounting. Record it
    // before later persistence or logging can fail or unwind, then derive the
    // remaining count from the same structured report used by the trailer.
    report.records.push(record.clone());
    report.pending = pending_count(total_expectations, &report.records, &report.cached_passes);
}
