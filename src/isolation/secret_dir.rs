use crate::platform::filesystem;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(super) enum SecretDirConfig {
    Absent,
    // `os.environ.get` returns the configured empty string, so it is not
    // `None` for the boundary assertion. Python's `if self.secret_dir` is
    // nevertheless false for that value, so it has no permission callback.
    Empty,
    Present {
        path: PathBuf,
        mode: filesystem::SecretDirMode,
    },
}

#[derive(Clone)]
pub(super) struct SecretDirModeRestoration {
    pub(super) path: PathBuf,
    pub(super) mode: filesystem::SecretDirMode,
}

impl SecretDirConfig {
    pub(super) fn from_path(path: Option<PathBuf>) -> Result<SecretDirConfig, String> {
        match path {
            None => Ok(SecretDirConfig::Absent),
            // Preserve presence and Python string truthiness as separate
            // properties instead of collapsing an empty value into `None`.
            Some(path) if path.as_os_str().is_empty() => Ok(SecretDirConfig::Empty),
            Some(path) => {
                let mode = filesystem::secret_dir_mode(&path)?;
                Ok(SecretDirConfig::Present { path, mode })
            }
        }
    }

    pub(super) fn boundary(&self) -> Option<&Path> {
        match self {
            SecretDirConfig::Absent => None,
            SecretDirConfig::Empty => Some(Path::new("")),
            SecretDirConfig::Present { path, .. } => Some(path),
        }
    }

    pub(super) fn permission_restoration(&self) -> Option<SecretDirModeRestoration> {
        match self {
            SecretDirConfig::Present { path, mode } => Some(SecretDirModeRestoration {
                path: path.clone(),
                mode: mode.clone(),
            }),
            // These are exactly the two cases where
            // `if self.secret_dir` is false in the policy pseudocode.
            SecretDirConfig::Absent | SecretDirConfig::Empty => None,
        }
    }
}

pub(super) fn chmod_secret_dir_no_access(path: &Path) -> Result<(), String> {
    filesystem::chmod_secret_dir_no_access(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: Hj
    fn empty_secret_dir_is_present_for_boundary_only() {
        let secret_dir = SecretDirConfig::from_path(Some(PathBuf::new())).unwrap();

        assert_eq!(secret_dir.boundary(), Some(Path::new("")));
        assert!(secret_dir.permission_restoration().is_none());
    }
}
