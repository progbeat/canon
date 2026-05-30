use super::wait_for_app_server_child;
use std::ffi::OsString;
use std::fs;
#[cfg(windows)]
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

pub(crate) fn install_check_signal_handlers() -> Result<(), String> {
    Ok(())
}

pub(crate) fn prepare_app_server_command(_command: &mut Command) {}

pub(crate) fn terminate_app_server_child(child: &mut Child) -> Result<(), String> {
    child
        .kill()
        .map_err(|err| format!("failed to kill app-server child: {}", err))?;
    wait_for_app_server_child(child)?;
    Ok(())
}

pub(crate) fn mirror_evaluator_codex_home_file(source: &Path, target: &Path) -> Result<(), String> {
    fs::copy(source, target).map(|_| ()).map_err(|err| {
        format!(
            "failed to copy evaluator CODEX_HOME file {} from {}: {}",
            target.display(),
            source.display(),
            err
        )
    })
}

pub(crate) fn make_hook_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub(crate) fn set_materialized_file_permissions(_path: &Path, _mode: &str) -> Result<(), String> {
    Ok(())
}

pub(crate) fn set_materialized_dir_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub(crate) fn set_private_dir_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub(crate) fn create_materialized_symlink(target: &[u8], link: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let target = PathBuf::from(git_bytes_os_string(target.to_vec())?);
        std::os::windows::fs::symlink_file(&target, link).map_err(|err| {
            format!(
                "failed to symlink evaluator file {} to {}: {}",
                link.display(),
                target.display(),
                err
            )
        })
    }
    #[cfg(not(windows))]
    {
        fs::write(link, target).map_err(|err| {
            format!(
                "failed to write evaluator symlink placeholder {}: {}",
                link.display(),
                err
            )
        })
    }
}

pub(crate) fn hardlink_file_or_copy_symlink(source: &Path, target: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let metadata = fs::symlink_metadata(source).map_err(|err| {
            format!(
                "failed to inspect evaluator file {}: {}",
                source.display(),
                err
            )
        })?;
        if metadata.file_type().is_symlink() {
            let link_target = fs::read_link(source)
                .map_err(|err| format!("failed to read symlink {}: {}", source.display(), err))?;
            return std::os::windows::fs::symlink_file(&link_target, target).map_err(|err| {
                format!(
                    "failed to copy evaluator symlink {} to {}: {}",
                    source.display(),
                    target.display(),
                    err
                )
            });
        }
    }
    fs::hard_link(source, target).map_err(|err| {
        format!(
            "failed to hardlink evaluator scope file {} to {}: {}",
            source.display(),
            target.display(),
            err
        )
    })
}

pub(crate) fn create_private_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)
}

pub(crate) fn create_private_dir_all(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

pub(crate) fn open_file_for_append_without_following_symlink(
    path: &Path,
) -> Result<fs::File, String> {
    fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))
}

pub(crate) fn add_memory_backed_staged_snapshot_parent_candidates(_parents: &mut Vec<PathBuf>) {}

pub(crate) fn add_ordinary_staged_snapshot_parent_candidates(parents: &mut Vec<PathBuf>) {
    add_environment_temp_parent_candidates(parents);
}

fn add_environment_temp_parent_candidates(parents: &mut Vec<PathBuf>) {
    // Common RAM-backed paths are added by platform.rs before this function.
    // Non-Unix platforms have no portable mount table; if the host provides a
    // RAM-backed temp directory through standard temp environment variables,
    // try it before the platform fallback temp dir.
    for name in ["TMPDIR", "TEMP", "TMP"] {
        let Some(value) = std::env::var_os(name) else {
            continue;
        };
        if !value.is_empty() {
            push_unique_path(parents, PathBuf::from(value));
        }
    }
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

pub(crate) fn path_from_git_bytes(bytes: Vec<u8>) -> Result<PathBuf, String> {
    git_bytes_os_string(bytes).map(PathBuf::from)
}

pub(crate) fn git_path_bytes(path: &Path) -> Result<Vec<u8>, String> {
    Ok(path
        .to_str()
        .ok_or_else(|| format!("git path must be valid UTF-8: {}", path.display()))?
        .as_bytes()
        .to_vec())
}

pub(crate) fn os_string_from_bytes(bytes: Vec<u8>) -> Result<OsString, String> {
    git_bytes_os_string(bytes)
}

#[cfg(windows)]
fn git_bytes_os_string(bytes: Vec<u8>) -> Result<OsString, String> {
    if bytes.contains(&0) {
        return Err("Git paths must not contain NUL bytes".to_string());
    }
    match String::from_utf8(bytes) {
        Ok(path) => Ok(OsString::from(path)),
        Err(err) => Ok(OsString::from_wide(&surrogate_escaped_git_path(
            &err.into_bytes(),
        ))),
    }
}

#[cfg(windows)]
fn surrogate_escaped_git_path(bytes: &[u8]) -> Vec<u16> {
    bytes
        .iter()
        .map(|byte| match *byte {
            b'/' => std::path::MAIN_SEPARATOR as u16,
            b'.' | b'-' | b'_' | b' ' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' => u16::from(*byte),
            byte => 0xDC00 | u16::from(byte),
        })
        .collect()
}

#[cfg(not(windows))]
fn git_bytes_os_string(bytes: Vec<u8>) -> Result<OsString, String> {
    String::from_utf8(bytes).map(OsString::from).map_err(|err| {
        format!(
            "Git path bytes are not UTF-8: 0x{}",
            hex_bytes(err.as_bytes())
        )
    })
}

#[cfg(not(windows))]
fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut hex, byte| {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}
