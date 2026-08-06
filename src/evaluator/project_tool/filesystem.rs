use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Read, Take};
use std::path::{Component, Path, PathBuf};

const VISITED_FILE_LIMIT: usize = 10_000;

pub(super) fn validated_relative_path(value: &str) -> Result<PathBuf, String> {
    validated_relative_path_value(Path::new(value))
}

pub(super) fn validated_relative_path_value(path: &Path) -> Result<PathBuf, String> {
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => relative.push(value),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "project inspection path must stay relative: {}",
                    path.display()
                ));
            }
        }
    }
    reject_git_admin_path(&relative)?;
    Ok(relative)
}

pub(super) fn existing_entry_without_symlink_traversal(
    root: &Path,
    relative: &Path,
) -> Result<PathBuf, String> {
    existing_path_without_symlink_traversal(root, relative, true)
}

fn existing_path_without_symlink_traversal(
    root: &Path,
    relative: &Path,
    allow_final_symlink: bool,
) -> Result<PathBuf, String> {
    if !root.is_absolute() {
        return Err(format!(
            "project inspection root is invalid: {}",
            root.display()
        ));
    }
    let root_metadata = inspect_path_without_symlinks(root)?;
    if !root_metadata.is_dir() {
        return Err(format!(
            "project inspection root is invalid: {}",
            root.display()
        ));
    }
    let mut current = root.to_path_buf();
    let components = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value),
            _ => None,
        })
        .collect::<Vec<_>>();
    for (index, value) in components.iter().enumerate() {
        current.push(value);
        let metadata = inspect_path(&current)?;
        if metadata.file_type().is_symlink() {
            if allow_final_symlink && index + 1 == components.len() {
                return Ok(current);
            }
            return Err(format!(
                "project inspection does not follow symbolic links: {}",
                current.display()
            ));
        }
    }
    let canonical_root = resolve_path(root)?;
    let canonical_current = resolve_path(&current)?;
    let canonical_relative = canonical_current
        .strip_prefix(&canonical_root)
        .map_err(|_| {
            format!(
                "project inspection path escapes its declared root: {}",
                current.display()
            )
        })?;
    reject_git_admin_path(canonical_relative)?;
    Ok(current)
}

pub(super) fn require_file_entry(path: &Path) -> Result<(), String> {
    let metadata = inspect_path(path)?;
    if metadata.is_file() || metadata.file_type().is_symlink() {
        Ok(())
    } else {
        Err(format!(
            "project inspection path is not a file: {}",
            path.display()
        ))
    }
}

pub(super) fn walk_file_entries(
    root: &Path,
    relative: &Path,
    mut visit: impl FnMut(&Path) -> Result<bool, String>,
) -> Result<bool, String> {
    let start = existing_entry_without_symlink_traversal(root, relative)?;
    let metadata = inspect_path(&start)?;
    if metadata.is_file() || metadata.file_type().is_symlink() {
        return visit(&start).map(|continue_walk| !continue_walk);
    }
    if !metadata.is_dir() {
        return Err(format!(
            "project inspection path is not a file or directory: {}",
            start.display()
        ));
    }
    let mut directories = vec![start];
    let mut visited_files = 0usize;
    while let Some(directory) = directories.pop() {
        let mut entries = directory_entries(&directory)?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries.into_iter().rev() {
            let path = entry.path();
            if is_git_admin_component(&entry.file_name()) {
                continue;
            }
            let file_type = entry
                .file_type()
                .map_err(|err| filesystem_error("inspect", &path, err))?;
            if file_type.is_symlink() {
                // [90,KD] The entry itself is visible project content, but it
                // remains a leaf capability boundary and is never followed.
                visited_files += 1;
                if visited_files > VISITED_FILE_LIMIT || !visit(&path)? {
                    return Ok(true);
                }
                continue;
            }
            let metadata = entry
                .metadata()
                .map_err(|err| filesystem_error("inspect", &path, err))?;
            if metadata.is_dir() {
                directories.push(path);
            } else if metadata.is_file() {
                visited_files += 1;
                if visited_files > VISITED_FILE_LIMIT || !visit(&path)? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

pub(super) fn read_bounded(path: &Path, limit: u64) -> Result<(Vec<u8>, bool), String> {
    if inspect_path(path)?.file_type().is_symlink() {
        let target = fs::read_link(path).map_err(|err| filesystem_error("read", path, err))?;
        let mut bytes = target.to_string_lossy().into_owned().into_bytes();
        let truncated = bytes.len() as u64 > limit;
        bytes.truncate(limit as usize);
        return Ok((bytes, truncated));
    }
    let file = File::open(path).map_err(|err| filesystem_error("read", path, err))?;
    let mut reader: Take<File> = file.take(limit.saturating_add(1));
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|err| filesystem_error("read", path, err))?;
    let truncated = bytes.len() as u64 > limit;
    bytes.truncate(limit as usize);
    Ok((bytes, truncated))
}

fn reject_git_admin_path(path: &Path) -> Result<(), String> {
    if path
        .components()
        .any(|component| is_git_admin_component(component.as_os_str()))
    {
        Err("project inspection does not expose Git administrative files".to_string())
    } else {
        Ok(())
    }
}

fn inspect_path(path: &Path) -> Result<fs::Metadata, String> {
    fs::symlink_metadata(path).map_err(|err| filesystem_error("inspect", path, err))
}

fn inspect_path_without_symlinks(path: &Path) -> Result<fs::Metadata, String> {
    let mut current = PathBuf::new();
    let mut final_metadata = None;
    for component in path.components() {
        current.push(component.as_os_str());
        if !matches!(component, Component::Normal(_)) {
            continue;
        }
        let metadata = inspect_path(&current)?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "project inspection does not follow symbolic links: {}",
                current.display()
            ));
        }
        final_metadata = Some(metadata);
    }
    final_metadata.map_or_else(|| inspect_path(path), Ok)
}

fn resolve_path(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|err| filesystem_error("resolve", path, err))
}

fn directory_entries(path: &Path) -> Result<Vec<fs::DirEntry>, String> {
    fs::read_dir(path)
        .and_then(|entries| entries.collect())
        .map_err(|err| filesystem_error("list", path, err))
}

fn filesystem_error(operation: &str, path: &Path, error: io::Error) -> String {
    format!("failed to {operation} {}: {error}", path.display())
}

fn is_git_admin_component(component: &OsStr) -> bool {
    component.to_str().is_some_and(|component| {
        component
            .trim_end_matches(['.', ' '])
            .eq_ignore_ascii_case(".git")
    })
}

#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;
    use crate::platform::filesystem::{
        OwnedPrivateTemporaryDirectory, PrivateTemporaryDirectoryAllocator,
    };
    use std::os::unix::fs::symlink;

    #[test] // xpec: bP,KD,hQ,qv
    fn project_roots_and_paths_never_follow_symbolic_links() {
        let temporary = OwnedPrivateTemporaryDirectory::create(
            &PrivateTemporaryDirectoryAllocator::new(),
            "canon-project-filesystem-test",
        )
        .unwrap();
        let project = temporary.path().join("project");
        let linked_project = temporary.path().join("linked-project");
        let linked_parent = temporary.path().join("linked-parent");
        fs::create_dir(&project).unwrap();
        symlink(&project, &linked_project).unwrap();
        symlink(temporary.path(), &linked_parent).unwrap();
        symlink("/etc/passwd", project.join("escape")).unwrap();

        let path_result =
            existing_path_without_symlink_traversal(&project, Path::new("escape"), false);
        let root_result =
            existing_path_without_symlink_traversal(&linked_project, Path::new("escape"), false);
        let ancestor_result = existing_path_without_symlink_traversal(
            &linked_parent.join("project"),
            Path::new("escape"),
            false,
        );

        assert!(path_result.is_err());
        assert!(path_result
            .unwrap_err()
            .contains("does not follow symbolic links"));
        assert!(root_result.is_err());
        assert!(root_result
            .unwrap_err()
            .contains("does not follow symbolic links"));
        assert!(ancestor_result.is_err());
        assert!(ancestor_result
            .unwrap_err()
            .contains("does not follow symbolic links"));
    }
}
