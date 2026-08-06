//! Filesystem policy for evaluator runtime files.

use crate::platform::filesystem::{
    OwnedPrivateTemporaryDirectory, PrivateTemporaryDirectoryAllocator,
};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod portable;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(not(target_os = "linux"))]
use portable as imp;

pub(crate) fn allocate_evaluator_runtime_directory(
    allocator: &PrivateTemporaryDirectoryAllocator,
    prefix: &str,
) -> Result<OwnedPrivateTemporaryDirectory, String> {
    imp::allocate(allocator, prefix)
}
