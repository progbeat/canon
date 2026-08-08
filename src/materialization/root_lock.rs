//! Cross-process ownership of a caller-provided materialization root.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

use std::path::Path;

pub(super) struct MaterializationRootLock {
    _inner: imp::MaterializationRootLock,
}

impl MaterializationRootLock {
    pub(super) fn acquire(root: &Path) -> Result<Self, String> {
        imp::MaterializationRootLock::acquire(root).map(|inner| Self { _inner: inner })
    }
}
