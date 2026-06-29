mod args;
mod cache_select;
mod cooldown;
mod identity;
mod order;

pub(crate) use args::{
    add_check_option_args, matched_os_values, raw_check_options_from_matches,
    resolve_check_options_with_identities,
};
pub(crate) use cache_select::{
    select_expectations_after_cache, CacheFilterContext, CachedExpectationHit, CachedFailureMode,
};
pub(crate) use cooldown::parse_cooldown;
// Selector parsing and matching, including `not:<ID-PREFIX>` exclusions, lives
// in `identity`.
pub(crate) use identity::{
    expectation_identities, minimal_unique_expectation_prefix, select_expectations_with_identities,
    selected_expectation_at, ExpectationIdentity,
};
pub(crate) use order::order_by_latest_non_pass;
