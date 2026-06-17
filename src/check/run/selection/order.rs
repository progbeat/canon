use crate::check::core::SelectedExpectation;
use crate::xpec_state::{latest_non_pass_timestamp, XpecStateCache};
use std::path::Path;

const UNIX_EPOCH_TIMESTAMP: u64 = 0;

pub(crate) fn order_by_latest_non_pass<T>(
    root: &Path,
    items: Vec<T>,
    state_cache: &mut XpecStateCache,
    expectation: impl Fn(&T) -> &SelectedExpectation,
) -> Result<Vec<T>, String> {
    let mut ordered = items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let latest = latest_non_pass_timestamp(root, expectation(&item), state_cache)?
                .unwrap_or(UNIX_EPOCH_TIMESTAMP);
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
