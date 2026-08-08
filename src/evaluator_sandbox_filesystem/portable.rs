//! Portable evaluator runtime filesystem policy.

use crate::platform::filesystem::{
    OwnedPrivateTemporaryDirectory, PrivateTemporaryDirectoryAllocator,
};

pub(super) fn allocate(
    allocator: &PrivateTemporaryDirectoryAllocator,
    prefix: &str,
) -> Result<OwnedPrivateTemporaryDirectory, String> {
    OwnedPrivateTemporaryDirectory::create_executable(allocator, prefix, None)
}
