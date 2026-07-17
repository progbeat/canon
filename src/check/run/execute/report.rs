use crate::check::core::{for_each_unique_report_record, CheckRecord, CheckRunReport};

pub(super) fn check_run_report(
    records: Vec<CheckRecord>,
    cached: Vec<CheckRecord>,
    counts: CheckRunReportCounts,
) -> CheckRunReport {
    CheckRunReport {
        records,
        cached,
        skipped: counts.skipped,
    }
}

pub(super) struct CheckRunReportCounts {
    pub(super) skipped: usize,
}

pub(crate) fn skipped_count(
    total_expectations: usize,
    records: &[CheckRecord],
    cached: &[CheckRecord],
) -> usize {
    let mut unique_records = 0usize;
    for_each_unique_report_record(records, cached, |_| unique_records += 1);
    total_expectations.saturating_sub(unique_records)
}
