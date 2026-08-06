use super::private_directory::create_private_dir;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

pub(crate) struct OwnedPrivateTemporaryDirectory {
    path: PathBuf,
}

#[derive(Clone)]
pub(crate) struct PrivateTemporaryDirectoryAllocator {
    candidates: Arc<OnceLock<imp::TemporaryParentCandidates>>,
}

impl PrivateTemporaryDirectoryAllocator {
    pub(crate) fn new() -> PrivateTemporaryDirectoryAllocator {
        PrivateTemporaryDirectoryAllocator {
            candidates: Arc::new(OnceLock::new()),
        }
    }

    fn candidates(&self) -> &imp::TemporaryParentCandidates {
        self.candidates
            .get_or_init(imp::temporary_parent_candidates)
    }
}

impl OwnedPrivateTemporaryDirectory {
    pub(crate) fn create(
        allocator: &PrivateTemporaryDirectoryAllocator,
        prefix: &str,
    ) -> Result<OwnedPrivateTemporaryDirectory, String> {
        Self::create_with_parents(
            prefix,
            candidate_parents_in_preference_order(allocator.candidates()),
        )
    }

    pub(crate) fn create_executable(
        allocator: &PrivateTemporaryDirectoryAllocator,
        prefix: &str,
        explicit_parent_candidates: Option<&[PathBuf]>,
    ) -> Result<OwnedPrivateTemporaryDirectory, String> {
        let candidates = allocator.candidates();
        let default_parents = explicit_parent_candidates
            .is_none()
            .then(|| candidate_parents_in_preference_order(candidates));
        let parents = explicit_parent_candidates
            .into_iter()
            .flatten()
            .chain(default_parents.into_iter().flatten());
        Self::create_with_executable_parents(candidates, prefix, parents)
    }

    fn create_with_executable_parents<'a>(
        candidates: &'a imp::TemporaryParentCandidates,
        prefix: &str,
        parents: impl IntoIterator<Item = &'a PathBuf>,
    ) -> Result<OwnedPrivateTemporaryDirectory, String> {
        Self::create_with_parents(
            prefix,
            parents
                .into_iter()
                .filter(|parent| candidates.allows_executables(parent)),
        )
    }

    #[cfg(test)]
    pub(crate) fn create_with_parent_candidates(
        prefix: &str,
        memory_backed_candidates: &[PathBuf],
        fallback_candidates: &[PathBuf],
    ) -> Result<OwnedPrivateTemporaryDirectory, String> {
        Self::create_with_parents(
            prefix,
            memory_backed_candidates.iter().chain(fallback_candidates),
        )
    }

    fn create_with_parents<'a>(
        prefix: &str,
        parents: impl IntoIterator<Item = &'a PathBuf>,
    ) -> Result<OwnedPrivateTemporaryDirectory, String> {
        let mut errors = Vec::new();
        for parent in parents {
            let parent = match imp::canonical_temporary_parent(parent) {
                Ok(parent) => parent,
                Err(err) => {
                    errors.push(format!(
                        "failed to resolve temporary directory parent {}: {}",
                        parent.display(),
                        err
                    ));
                    continue;
                }
            };
            for _ in 0..64 {
                let random = getrandom::u64().map_err(|err| {
                    format!("failed to choose private temporary directory: {err}")
                })?;
                let path = parent.join(format!("{prefix}-{}-{random:016x}", std::process::id()));
                match create_private_dir(&path) {
                    Ok(()) => return Ok(OwnedPrivateTemporaryDirectory { path }),
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(err) => {
                        errors.push(format!("failed to create {}: {}", path.display(), err));
                        break;
                    }
                }
            }
        }
        Err(format!(
            "failed to allocate private temporary directory: {}",
            errors.join("; ")
        ))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

fn candidate_parents_in_preference_order(
    candidates: &imp::TemporaryParentCandidates,
) -> impl Iterator<Item = &PathBuf> {
    candidates
        .memory_backed()
        .iter()
        .chain(candidates.fallback())
}

impl Drop for OwnedPrivateTemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(super) fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

pub(crate) fn resolve_standard_temporary_path(path: &Path) -> PathBuf {
    imp::resolve_standard_temporary_path(path)
}
