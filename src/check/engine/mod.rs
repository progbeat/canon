// A check run owns expectation selection, cache reuse, and per-expectation
// execution. The submodules track those runtime concerns.
pub(super) mod cache;
mod execute;
pub(super) mod selection;

pub(crate) use execute::{
    run_check_with_runner_and_caches, run_temporary_expectation_interrogation, CheckRunSideEffects,
    ResolveSelectedDiffFromTreeOids, TemporaryExpectationInterrogationContext,
};
pub(crate) use selection::{
    expectation_identities, minimal_unique_expectation_prefix,
    order_selected_by_rank_and_latest_fail, select_expectations_with_identities,
    ExpectationIdentity,
};
