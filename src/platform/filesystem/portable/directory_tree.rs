use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
pub(super) mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

pub(super) trait DirectoryRegistrar {
    type Error;

    fn register(&mut self, path: &Path, metadata: &fs::Metadata) -> Result<(), Self::Error>;
    fn filesystem_error(&self, context: String, source: io::Error) -> Self::Error;
}

/// One pass of directory registration using a fixed strategy.
///
/// `extend` invokes the registrar while it walks. A failed walk may therefore
/// leave the registrar partially populated; a registrar that mutates live
/// directories must own the corresponding rollback. This is a one-shot
/// mutation primitive, not reusable snapshot capture.
pub(super) struct DirectoryTreeRegistration<R> {
    paths: BTreeSet<PathBuf>,
    registrar: R,
}

impl<R> DirectoryTreeRegistration<R>
where
    R: DirectoryRegistrar,
{
    pub(super) fn new(registrar: R) -> Self {
        DirectoryTreeRegistration {
            paths: BTreeSet::new(),
            registrar,
        }
    }

    #[cfg(unix)]
    pub(super) fn registrar_mut(&mut self) -> &mut R {
        &mut self.registrar
    }

    #[cfg(unix)]
    pub(super) fn into_registrar(self) -> R {
        self.registrar
    }

    pub(super) fn extend(&mut self, path: &Path) -> Result<(), R::Error> {
        if self.paths.contains(path) {
            return Ok(());
        }
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(self.registrar.filesystem_error(
                    format!("failed to inspect directory {}", path.display()),
                    source,
                ));
            }
        };
        if !metadata.file_type().is_dir() {
            return Ok(());
        }
        self.registrar.register(path, &metadata)?;
        self.paths.insert(path.to_path_buf());

        let entries = fs::read_dir(path)
            .and_then(|entries| entries.collect::<io::Result<Vec<_>>>())
            .map_err(|source| {
                self.registrar.filesystem_error(
                    format!("failed to read directory {}", path.display()),
                    source,
                )
            })?;
        for entry in entries {
            self.extend(&entry.path())?;
        }
        Ok(())
    }
}

pub(crate) fn make_directory_tree_private(path: &Path) -> Result<(), String> {
    imp::make_directory_tree_private(path).map_err(super::filesystem_error)
}
