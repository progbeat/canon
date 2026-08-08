use crate::check::core::ResolvedExpectation;
use crate::xpec_state::{latest_fail_timestamp, XpecStateCache};
use std::cmp::Reverse;
use std::convert::Infallible;
use std::path::Path;

const UNIX_EPOCH_TIMESTAMP: u64 = 0;

pub(crate) fn order_selected_by_rank_and_latest_fail<T>(
    root: &Path,
    items: Vec<T>,
    last_result_history: &mut XpecStateCache,
    expectation: impl Fn(&T) -> &ResolvedExpectation,
) -> Result<Vec<T>, String> {
    // [H9] The caller passes selected evaluator work after applying the
    // mode-specific selection policy; this function only orders that work and
    // never turns history into cached results or removes an item.
    order_selected_by_fields(items, expectation, |expectation| {
        Ok(
            latest_fail_timestamp(root, expectation, last_result_history)?
                .unwrap_or(UNIX_EPOCH_TIMESTAMP),
        )
    })
}

pub(crate) fn order_selected_when_every_expectation_has_no_fail_result<T>(
    items: Vec<T>,
    expectation: impl Fn(&T) -> &ResolvedExpectation,
) -> Vec<T> {
    order_selected_by_fields(items, expectation, |_| {
        Ok::<u64, Infallible>(UNIX_EPOCH_TIMESTAMP)
    })
    .unwrap_or_else(|never| match never {})
}

fn order_selected_by_fields<T, E>(
    items: Vec<T>,
    expectation: impl Fn(&T) -> &ResolvedExpectation,
    mut latest_fail: impl FnMut(&ResolvedExpectation) -> Result<u64, E>,
) -> Result<Vec<T>, E> {
    let items_with_order_fields = items
        .into_iter()
        .map(|item| {
            let expectation = expectation(&item);
            let rank = expectation.rank;
            let latest_fail = latest_fail(expectation)?;
            Ok((item, rank, latest_fail))
        })
        .collect::<Result<Vec<_>, E>>()?;
    Ok(order_precomputed_by_rank_and_latest_fail(
        items_with_order_fields,
    ))
}

fn order_precomputed_by_rank_and_latest_fail<T>(items: Vec<(T, i64, u64)>) -> Vec<T> {
    let mut ordered = items
        .into_iter()
        .enumerate()
        .map(|(index, (item, rank, latest_fail))| {
            let key = CheckOrderKey {
                rank_ascending: rank,
                latest_fail_descending: Reverse(latest_fail),
                original_selection_index: index,
            };
            (key, item)
        })
        .collect::<Vec<_>>();
    ordered.sort_by_key(|(key, _)| *key);
    ordered.into_iter().map(|(_, item)| item).collect()
}

// xpec: H9
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CheckOrderKey {
    rank_ascending: i64,
    latest_fail_descending: Reverse<u64>,
    original_selection_index: usize,
}

#[cfg(test)]
mod tests {
    use super::{
        order_precomputed_by_rank_and_latest_fail,
        order_selected_when_every_expectation_has_no_fail_result,
    };
    use crate::check::core::ResolvedExpectation;
    use crate::config_types::{AgentConfig, ExpectationTo, DEFAULT_DIFF_FROM};

    #[test] // xpec: H9
    fn rank_precedes_latest_fail_and_ties_remain_stable() {
        let items = vec![
            ("rank-zero-old", 0, 1),
            ("rank-negative", -1, 0),
            ("rank-zero-new", 0, 2),
            ("rank-zero-new-tie", 0, 2),
        ];

        let names = order_precomputed_by_rank_and_latest_fail(items);

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

    #[test] // xpec: H9,90
    fn absent_latest_fail_results_all_use_epoch_and_keep_same_rank_ties_stable() {
        let expectations = vec![
            expectation("later-rank", 1),
            expectation("same-rank-first", 0),
            expectation("same-rank-second", 0),
            expectation("earlier-rank", -1),
        ];

        let ordered =
            order_selected_when_every_expectation_has_no_fail_result(expectations, |expectation| {
                expectation
            });

        assert_eq!(
            ordered
                .into_iter()
                .map(|expectation| expectation.require_configured_id().unwrap().to_string())
                .collect::<Vec<_>>(),
            vec![
                "earlier-rank",
                "same-rank-first",
                "same-rank-second",
                "later-rank",
            ]
        );
    }

    fn expectation(id: &str, rank: i64) -> ResolvedExpectation {
        ResolvedExpectation {
            kind: crate::check::core::ResolvedExpectationKind::Configured { id: id.to_string() },
            display_id: id.to_string(),
            to: ExpectationTo::Agent,
            rank,
            question: String::new(),
            expected_answer: "yes".to_string(),
            question_context: String::new(),
            diff_from: DEFAULT_DIFF_FROM.to_string(),
            target: None,
            agent: AgentConfig::default(),
            cooldown: None,
            q_scope: Default::default(),
        }
    }
}
