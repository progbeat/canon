use crate::check::core::ResolvedExpectation;
use crate::xpec_state::{latest_non_pass_timestamp, XpecStateCache};
use std::path::Path;

const UNIX_EPOCH_TIMESTAMP: u64 = 0;

pub(crate) fn order_by_latest_non_pass<T>(
    root: &Path,
    items: Vec<T>,
    state_cache: &mut XpecStateCache,
    expectation: impl Fn(&T) -> &ResolvedExpectation,
) -> Result<Vec<T>, String> {
    // The caller passes only work that remains after selection/cache policy:
    // cached report blocks plus final selected evaluator work. Ordering is
    // purely by each expectation's latest non-pass timestamp.
    order_by_latest_non_pass_with(items, |item| {
        latest_non_pass_timestamp(root, expectation(item), state_cache)
            .map(|latest| latest.unwrap_or(UNIX_EPOCH_TIMESTAMP))
    })
}

pub(crate) fn order_in_place_by_absent_non_pass_history<T>(items: Vec<T>) -> Vec<T> {
    // Canon check --in-place treats persisted xpec last-result history as
    // absent. Under e5, an expectation with no non-pass result uses the Unix
    // epoch, so every in-place selected expectation has the same ordering key
    // and the stable tie-breaker preserves candidate order without state reads.
    order_by_latest_non_pass_with(items, |_| Ok(UNIX_EPOCH_TIMESTAMP))
        .expect("absent non-pass history ordering is infallible")
}

fn order_by_latest_non_pass_with<T>(
    items: Vec<T>,
    mut latest_for: impl FnMut(&T) -> Result<u64, String>,
) -> Result<Vec<T>, String> {
    let mut ordered = items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let latest = latest_for(&item)?;
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
