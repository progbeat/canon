mod args;
mod cache_select;
mod identity;
mod order;

pub(crate) use args::{
    add_check_option_args, matched_os_values, raw_check_options_from_matches,
    resolve_check_options_with_identities,
};
pub(crate) use cache_select::{
    select_and_order_git_backed_expectations, GitBackedCacheFilterContext,
};
// Selector parsing and matching, including `not:<ID-PREFIX>` exclusions, lives
// in `identity`.
pub(crate) use identity::{
    expectation_identities, minimal_unique_expectation_prefix, select_expectations_with_identities,
    ExpectationIdentity,
};
pub(crate) use order::{order_by_latest_fail, order_in_place_by_absent_fail_history};
