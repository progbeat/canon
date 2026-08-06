mod raw;
mod resolved;

pub(crate) use raw::{
    CooldownConfig, QScopeConfig, RawExpectationCommonConfig, RawExpectationFields,
    RawExpectationItem, RawGitBackedExpectationConfig,
};
pub(crate) use resolved::{
    Cooldown, Expectation, ExpectationTarget, ExpectationTo, InPlaceIncompatibleField, QScope,
    AGAINST_TREE_DIFF_FROM, DEFAULT_DIFF_FROM,
};
