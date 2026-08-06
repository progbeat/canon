const DEFAULT_EXPECTATION_RANK: i64 = 0;

pub(super) fn resolve_expectation_rank(configured_rank: Option<i64>) -> i64 {
    configured_rank.unwrap_or(DEFAULT_EXPECTATION_RANK)
}

#[cfg(test)]
mod tests {
    use super::resolve_expectation_rank;

    #[test] // xpec: H9
    fn omitted_rank_defaults_to_zero_and_configured_rank_is_preserved() {
        assert_eq!(resolve_expectation_rank(None), 0);
        assert_eq!(resolve_expectation_rank(Some(-3)), -3);
    }
}
