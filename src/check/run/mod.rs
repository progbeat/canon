// A check run owns expectation selection, cache reuse, and per-expectation
// execution. The submodules track those runtime concerns.
pub(super) mod cache;
mod execute;
pub(super) mod selection;

pub(crate) use execute::{
    run_check_with_runner_and_caches, skipped_count, CheckRunCaches, CheckRunSideEffects,
};
pub(crate) use selection::{
    expectation_identities, select_expectations_with_identities, ExpectationIdentity,
};
