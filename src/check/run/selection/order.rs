use crate::check::core::ResolvedExpectation;
use crate::xpec_state::{latest_fail_timestamp, XpecStateCache};
use std::path::Path;

const UNIX_EPOCH_TIMESTAMP: u64 = 0;

pub(crate) fn order_by_latest_fail<T>(
    root: &Path,
    items: Vec<T>,
    state_cache: &mut XpecStateCache,
    expectation: impl Fn(&T) -> &ResolvedExpectation,
) -> Result<Vec<T>, String> {
    // The caller passes only selected evaluator work after cache filtering.
    order_by_latest_fail_with(items, |item| {
        let expectation = expectation(item);
        latest_fail_timestamp(root, expectation, state_cache)
            .map(|latest| (expectation.rank, latest.unwrap_or(UNIX_EPOCH_TIMESTAMP)))
    })
}

pub(crate) fn order_in_place_by_absent_fail_history<T>(
    items: Vec<T>,
    expectation: impl Fn(&T) -> &ResolvedExpectation,
) -> Vec<T> {
    // Canon check --in-place treats persisted xpec last-result history as
    // absent. Under the order policy, an expectation with no fail result uses the Unix
    // epoch, so every in-place selected expectation has the same ordering key
    // and the stable tie-breaker preserves candidate order without state reads.
    order_by_latest_fail_with(items, |item| {
        Ok((expectation(item).rank, UNIX_EPOCH_TIMESTAMP))
    })
    .expect("absent non-pass history ordering is infallible")
}

fn order_by_latest_fail_with<T>(
    items: Vec<T>,
    mut key_for: impl FnMut(&T) -> Result<(i64, u64), String>,
) -> Result<Vec<T>, String> {
    let mut ordered = items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let (rank, latest) = key_for(&item)?;
            Ok(OrderedByLatestFail {
                item,
                rank,
                latest,
                index,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    ordered.sort_by(|left, right| {
        left.rank
            .cmp(&right.rank)
            .then_with(|| right.latest.cmp(&left.latest))
            .then_with(|| left.index.cmp(&right.index))
    });
    Ok(ordered.into_iter().map(|ordered| ordered.item).collect())
}

struct OrderedByLatestFail<T> {
    item: T,
    rank: i64,
    latest: u64,
    index: usize,
}

#[cfg(test)]
mod tests {
    use super::order_by_latest_fail_with;

    #[test] // xpec: Un
    fn rank_precedes_latest_fail_and_ties_remain_stable() {
        let items = vec![
            ("rank-zero-old", 0, 1),
            ("rank-negative", -1, 0),
            ("rank-zero-new", 0, 2),
            ("rank-zero-new-tie", 0, 2),
        ];

        let ordered = order_by_latest_fail_with(items, |item| Ok((item.1, item.2)))
            .expect("order selected xpecs");
        let names = ordered.into_iter().map(|item| item.0).collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "rank-negative",
                "rank-zero-new",
                "rank-zero-new-tie",
                "rank-zero-old",
            ]
        );
    }
}
