use crate::check::core::types::{
    CheckRecord, CheckResult, ObservedAnswerState, SelectedExpectation,
};
use crate::check::run::order_state::latest_recorded_non_pass_timestamp_with_cache;
use crate::history::HistoryCache;
use crate::time::parse_record_timestamp;
use std::path::Path;

const UNIX_EPOCH_TIMESTAMP: u64 = 0;

pub(crate) fn order_expectations_by_latest_non_pass(
    root: &Path,
    selected: Vec<SelectedExpectation>,
    history_cache: &mut HistoryCache,
) -> Result<Vec<SelectedExpectation>, String> {
    let mut ordered = selected
        .into_iter()
        .enumerate()
        .map(|(index, expectation)| {
            let latest = latest_non_pass_timestamp_with_cache(root, &expectation, history_cache)?;
            Ok(OrderedExpectation {
                expectation,
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
    Ok(ordered
        .into_iter()
        .map(|ordered| ordered.expectation)
        .collect())
}

struct OrderedExpectation {
    expectation: SelectedExpectation,
    latest: u64,
    index: usize,
}

pub(crate) fn latest_non_pass_timestamp_with_cache(
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
        .filter(|record| history_record_is_non_pass(record, &expectation.a))
        .filter_map(|record| parse_record_timestamp(&record.timestamp))
        .max())
}

fn history_record_is_non_pass(record: &CheckRecord, expected: &str) -> bool {
    ObservedAnswerState::from_error(record.error.as_deref()).requires_human_review()
        || CheckResult::from_expected_answer(expected, &record.observed) == CheckResult::Fail
}
