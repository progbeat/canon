use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};

pub(crate) fn ensure_dir_without_symlinks(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                current.push(prefix.as_os_str());
                continue;
            }
            Component::RootDir => {
                current.push(component.as_os_str());
                continue;
            }
            Component::CurDir => continue,
            Component::ParentDir => {
                return Err(format!(
                    "refusing to create directory through parent component {}",
                    path.display()
                ));
            }
            Component::Normal(part) => current.push(part),
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(format!("refusing to use symlink {}", current.display()));
                }
                if !metadata.is_dir() {
                    return Err(format!(
                        "{} exists but is not a directory",
                        current.display()
                    ));
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => fs::create_dir(&current)
                .map_err(|err| format!("failed to create {}: {}", current.display(), err))?,
            Err(err) => {
                return Err(format!("failed to inspect {}: {}", current.display(), err));
            }
        }
    }
    Ok(())
}

pub(crate) fn reject_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("refusing to use symlink {}", path.display()))
        }
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("failed to inspect {}: {}", path.display(), err)),
    }
}

pub(crate) fn path_exists_no_follow(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(false)
        }
        Err(err) => Err(format!("failed to inspect {}: {}", path.display(), err)),
    }
}

pub(crate) fn ensure_project_dir_without_symlinks(root: &Path, path: &Path) -> Result<(), String> {
    path.strip_prefix(root).map_err(|_| {
        format!(
            "refusing to create directory outside project root: {}",
            path.display()
        )
    })?;
    ensure_dir_without_symlinks(path)
}

pub(crate) fn write_new_file(path: &Path, content: &str) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| format!("failed to create {}: {}", path.display(), err))?;
    file.write_all(content.as_bytes())
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    file.flush()
        .map_err(|err| format!("failed to flush {}: {}", path.display(), err))
}

pub(crate) fn replace_file(path: &Path, content: &str) -> Result<(), String> {
    remove_optional_file(path)?;
    write_new_file(path, content)
}

pub(crate) fn remove_optional_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(())
        }
        Err(err) => Err(format!("failed to remove {}: {}", path.display(), err)),
    }
}

pub(crate) fn for_each_nonempty_line(
    path: &Path,
    mut visit: impl FnMut(usize, String) -> Result<(), String>,
) -> Result<(), String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("failed to open {}: {}", path.display(), err)),
    };
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line_number = index + 1;
        let line = line.map_err(|err| {
            format!(
                "failed to read {} line {}: {}",
                path.display(),
                line_number,
                err
            )
        })?;
        if !line.trim().is_empty() {
            visit(line_number, line)?;
        }
    }
    Ok(())
}

pub(crate) fn replace_file_with_temp(temp_path: &Path, path: &Path) -> Result<(), String> {
    fs::rename(temp_path, path).map_err(|err| {
        let _ = fs::remove_file(temp_path);
        format!(
            "failed to replace {} with {}: {}",
            path.display(),
            temp_path.display(),
            err
        )
    })
}

pub(crate) fn write_temp_file_then_replace(
    temp_path: &Path,
    path: &Path,
    write_content: impl FnOnce(&mut fs::File) -> Result<(), String>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        ensure_dir_without_symlinks(parent)?;
    }
    reject_symlink(path)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp_path)
        .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
    write_content(&mut file)?;
    file.flush()
        .map_err(|err| format!("failed to flush {}: {}", temp_path.display(), err))?;
    drop(file);
    replace_file_with_temp(temp_path, path)
}

pub(crate) fn crossed_size_compaction_bucket(
    previous_size: u64,
    current_size: u64,
    min_size: u64,
) -> bool {
    size_compaction_bucket(current_size, min_size) > size_compaction_bucket(previous_size, min_size)
}

fn size_compaction_bucket(size: u64, min_size: u64) -> u32 {
    if min_size == 0 || size < min_size {
        return 0;
    }
    let units = size / min_size;
    u64::BITS - units.leading_zeros()
}
