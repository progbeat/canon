use crate::check::core::ResolvedExpectation;
use crate::xpec_state::{latest_fail_timestamp, XpecStateCache};
use std::cmp::Reverse;
use std::path::Path;

const UNIX_EPOCH_TIMESTAMP: u64 = 0;

pub(crate) fn order_selected_by_rank_and_latest_fail<T>(
    root: &Path,
    items: Vec<T>,
    last_result_history: &mut XpecStateCache,
    expectation: impl Fn(&T) -> &ResolvedExpectation,
) -> Result<Vec<T>, String> {
    // [cv] The caller passes selected evaluator work after applying the
    // mode-specific selection policy; this function only orders that work and
    // never turns history into cached results or removes an item.
    order_by_rank_and_latest_fail_with(items, |item| {
        let expectation = expectation(item);
        latest_fail_timestamp(root, expectation, last_result_history)
            .map(|latest| (expectation.rank, latest.unwrap_or(UNIX_EPOCH_TIMESTAMP)))
    })
}

fn order_by_rank_and_latest_fail_with<T>(
    items: Vec<T>,
    mut key_for: impl FnMut(&T) -> Result<(i64, u64), String>,
) -> Result<Vec<T>, String> {
    let mut ordered = items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let (rank, latest_fail) = key_for(&item)?;
            let key = CheckOrderKey {
                rank_ascending: rank,
                latest_fail_descending: Reverse(latest_fail),
                original_selection_index: index,
            };
            Ok((key, item))
        })
        .collect::<Result<Vec<_>, String>>()?;
    ordered.sort_by_key(|(key, _)| *key);
    Ok(ordered.into_iter().map(|(_, item)| item).collect())
}

// xpec: cv
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CheckOrderKey {
    rank_ascending: i64,
    latest_fail_descending: Reverse<u64>,
    original_selection_index: usize,
}

#[cfg(test)]
mod tests {
    use super::order_by_rank_and_latest_fail_with;

    #[test] // xpec: cv
    fn rank_precedes_latest_fail_and_ties_remain_stable() {
        let items = vec![
            ("rank-zero-old", 0, 1),
            ("rank-negative", -1, 0),
            ("rank-zero-new", 0, 2),
            ("rank-zero-new-tie", 0, 2),
        ];

        let ordered = order_by_rank_and_latest_fail_with(items, |item| Ok((item.1, item.2)))
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
