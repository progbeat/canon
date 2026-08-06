//! Persistent check-result lifecycle.
//!
//! `records` forwards completed evaluation records to the xpec-state component,
//! which owns both configuration retention and per-expectation writes.

mod records;

pub(in crate::check::engine::execute) use records::{
    persist_finished_check_expectation, FinishedCheckRecordSource,
};
