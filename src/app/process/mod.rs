//! The app-server child lifecycle and transport boundary.
//!
//! Runner owns the complete child lifecycle, app-server communication, and
//! accounting.

mod runner;

pub(crate) use runner::LazyAppServerRunner;
