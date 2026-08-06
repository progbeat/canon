//! Linux evaluator runtime filesystem policy.

use crate::platform::filesystem::{
    OwnedPrivateTemporaryDirectory, PrivateTemporaryDirectoryAllocator,
};
use std::path::PathBuf;

const SANDBOX_WRITABLE_TEMPORARY_ROOTS: [&str; 2] = ["/tmp", "/var/tmp"];

pub(super) fn allocate(
    allocator: &PrivateTemporaryDirectoryAllocator,
    prefix: &str,
) -> Result<OwnedPrivateTemporaryDirectory, String> {
    let parents = SANDBOX_WRITABLE_TEMPORARY_ROOTS
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    OwnedPrivateTemporaryDirectory::create_executable(allocator, prefix, Some(&parents))
}
