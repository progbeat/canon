use super::super::unix::{PlatformError, PlatformResult};
use super::{DirectoryRegistrar, DirectoryTreeRegistration};
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
struct DirectoryMode {
    path: PathBuf,
    mode: u32,
}

pub(in super::super) struct DirectoryTreeModeGuard {
    directories: Arc<Vec<DirectoryMode>>,
    changed_indices: Vec<usize>,
}

struct DirectoryModeRegistrar {
    directories: Arc<Vec<DirectoryMode>>,
    changed_indices: Vec<usize>,
}

impl DirectoryRegistrar for DirectoryModeRegistrar {
    type Error = PlatformError;

    fn register(&mut self, path: &Path, metadata: &fs::Metadata) -> PlatformResult<()> {
        let index = self.directories.len();
        Arc::make_mut(&mut self.directories).push(DirectoryMode {
            path: path.to_path_buf(),
            mode: metadata.permissions().mode(),
        });
        let directory = &self.directories[index];
        set_directory_mode(directory, 0o700).map_err(|err| {
            PlatformError::io(
                format!("failed to make directory private {}", path.display()),
                err,
            )
        })?;
        self.changed_indices.push(index);
        Ok(())
    }

    fn filesystem_error(&self, context: String, source: io::Error) -> PlatformError {
        PlatformError::io(context, source)
    }
}

impl DirectoryModeRegistrar {
    fn restore_after_error(&mut self, error: PlatformError) -> PlatformError {
        restore_after_error(&self.directories, &mut self.changed_indices, error)
    }
}

impl DirectoryTreeModeGuard {
    pub(in super::super) fn restore(mut self) -> PlatformResult<()> {
        self.restore_once()
    }

    fn restore_once(&mut self) -> PlatformResult<()> {
        restore_directory_modes(&self.directories, &mut self.changed_indices)
    }
}

impl Drop for DirectoryTreeModeGuard {
    fn drop(&mut self) {
        let _ = self.restore_once();
    }
}

pub(in super::super) fn make_directory_tree_private_with_restore(
    path: &Path,
) -> PlatformResult<DirectoryTreeModeGuard> {
    let mut registration = DirectoryTreeRegistration::new(DirectoryModeRegistrar {
        directories: Arc::new(Vec::new()),
        changed_indices: Vec::new(),
    });
    if let Err(error) = registration.extend(path) {
        return Err(registration.registrar_mut().restore_after_error(error));
    }
    let registrar = registration.into_registrar();
    Ok(DirectoryTreeModeGuard {
        directories: registrar.directories,
        changed_indices: registrar.changed_indices,
    })
}

pub(in super::super) fn make_directory_tree_private(path: &Path) -> PlatformResult<()> {
    let mut guard = make_directory_tree_private_with_restore(path)?;
    guard.changed_indices.clear();
    Ok(())
}

fn set_directory_mode(directory: &DirectoryMode, mode: u32) -> io::Result<()> {
    fs::set_permissions(&directory.path, fs::Permissions::from_mode(mode))
}

fn restore_after_error(
    directories: &[DirectoryMode],
    changed_indices: &mut Vec<usize>,
    primary_error: PlatformError,
) -> PlatformError {
    match restore_directory_modes(directories, changed_indices) {
        Ok(()) => primary_error,
        Err(restore_error) => PlatformError::chain(vec![primary_error, restore_error]),
    }
}

fn restore_directory_modes(
    directories: &[DirectoryMode],
    changed_indices: &mut Vec<usize>,
) -> PlatformResult<()> {
    let mut errors = Vec::new();
    for index in changed_indices.drain(..).rev() {
        let directory = &directories[index];
        match set_directory_mode(directory, directory.mode) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => errors.push(PlatformError::io(
                format!(
                    "failed to restore directory permissions {}",
                    directory.path.display()
                ),
                err,
            )),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(PlatformError::chain(errors))
    }
}
