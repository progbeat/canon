use crate::platform::filesystem;
use std::path::PathBuf;

pub(super) fn relative_path_from_git_path(path: &[u8]) -> Result<PathBuf, String> {
    let path = PathBuf::from(filesystem::os_string_from_bytes(path.to_vec())?);
    if path.is_absolute() {
        return Err(format!(
            "Git tree entry path must be relative: {}",
            path.display()
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!(
            "Git tree entry path must not contain '..': {}",
            path.display()
        ));
    }
    Ok(path)
}
