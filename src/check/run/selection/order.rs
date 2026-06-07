use crate::check::core::{CheckRecord, CheckResult, SelectedExpectation};
use crate::check::run::order_state::latest_recorded_non_pass_timestamp_with_cache;
use crate::history::HistoryCache;
use crate::time::parse_record_timestamp;
use std::path::Path;

const UNIX_EPOCH_TIMESTAMP: u64 = 0;

pub(crate) fn order_by_latest_non_pass<T>(
    root: &Path,
    items: Vec<T>,
    history_cache: &mut HistoryCache,
    expectation: impl Fn(&T) -> &SelectedExpectation,
) -> Result<Vec<T>, String> {
    let mut ordered = items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let latest =
                latest_non_pass_timestamp_with_cache(root, expectation(&item), history_cache)?;
            Ok(OrderedByLatestNonPass {
                item,
                latest,
                index,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    ordered.sort_by(|left, right| {
        right
            .latest
            .cmp(&left.latest)
            .then_with(|| left.index.cmp(&right.index))
    });
    Ok(ordered.into_iter().map(|ordered| ordered.item).collect())
}

struct OrderedByLatestNonPass<T> {
    item: T,
    latest: u64,
    index: usize,
}

fn latest_non_pass_timestamp_with_cache(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<u64, String> {
    let latest = latest_history_non_pass_timestamp(root, expectation, history_cache)?
        .into_iter()
        .chain(latest_recorded_non_pass_timestamp_with_cache(
            root,
            expectation,
            history_cache,
        )?)
        .max();
    Ok(latest.unwrap_or(UNIX_EPOCH_TIMESTAMP))
}

fn latest_history_non_pass_timestamp(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<Option<u64>, String> {
    Ok(history_cache
        .read_records(root, expectation)?
        .into_iter()
        .filter(|record| history_record_is_non_pass(record, &expectation.expected_answer))
        .filter_map(|record| parse_record_timestamp(&record.timestamp))
        .max())
}

fn history_record_is_non_pass(record: &CheckRecord, expected: &str) -> bool {
    record.error.is_some()
        || CheckResult::from_expected_answer(expected, &record.observed) == CheckResult::Fail
}
