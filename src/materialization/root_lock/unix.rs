use std::fs::File;
use std::path::Path;

pub(super) struct MaterializationRootLock {
    _directory: File,
}

impl MaterializationRootLock {
    pub(super) fn acquire(root: &Path) -> Result<Self, String> {
        let directory = File::open(root).map_err(|err| {
            format!(
                "failed to open materialization root {} for locking: {}",
                root.display(),
                err
            )
        })?;
        directory.lock().map_err(|err| {
            format!(
                "failed to lock caller-owned materialization root {}: {}",
                root.display(),
                err
            )
        })?;
        Ok(Self {
            _directory: directory,
        })
    }
}
