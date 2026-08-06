//! Codex app-server integration.
//!
//! `process` owns child lifecycle and runner assembly, while `protocol` owns
//! wire values.

mod process;
mod protocol;

pub(crate) use process::LazyAppServerRunner;
