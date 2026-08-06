//! Portable platform-service components.
//!
//! Filesystem and process behavior each own their portable facade, shared
//! state, and operating-system implementations.

#[cfg(not(any(unix, windows)))]
compile_error!("canon requires Unix or Windows filesystem support");

pub(crate) mod filesystem;
pub(crate) mod process;
