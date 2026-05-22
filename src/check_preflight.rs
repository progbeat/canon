use crate::CHECK_INTERRUPTED;
#[cfg(unix)]
use crate::{handle_sigint, signal};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::Ordering;

pub(crate) fn install_sigint_handler() -> Result<(), String> {
    #[cfg(unix)]
    {
        const SIGHUP: i32 = 1;
        const SIGINT: i32 = 2;
        const SIGTERM: i32 = 15;
        install_signal_handler(SIGHUP)?;
        install_signal_handler(SIGINT)?;
        install_signal_handler(SIGTERM)?;
    }
    Ok(())
}

#[cfg(unix)]
fn install_signal_handler(signal_number: i32) -> Result<(), String> {
    const SIG_ERR: usize = usize::MAX;
    // SAFETY: `handle_sigint` has C ABI and only sets an atomic flag, so it is
    // valid to register as a simple process signal handler. We check for the
    // platform SIG_ERR sentinel immediately after the FFI call.
    let previous = unsafe { signal(signal_number, handle_sigint) };
    if previous == SIG_ERR {
        Err(format!(
            "failed to install signal handler for signal {}",
            signal_number
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn check_interrupted() -> bool {
    CHECK_INTERRUPTED.load(Ordering::SeqCst)
}

#[cfg(test)]
pub(crate) fn staged_changed_paths(root: &Path) -> Result<Vec<String>, String> {
    Ok(staged_changed_path_bytes(root)?
        .into_iter()
        .map(|path| String::from_utf8_lossy(&path).into_owned())
        .collect())
}

pub(crate) fn staged_changed_path_bytes(root: &Path) -> Result<Vec<Vec<u8>>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("diff")
        .arg("--cached")
        .arg("--name-status")
        .arg("-z")
        // Gate classification needs every staged path, including deletions
        // under `.canon/**`, so do not restrict diff status codes here.
        .output()
        .map_err(|err| format!("failed to run git diff: {}", err))?;
    if !output.status.success() {
        return Err("failed to inspect staged git changes".to_string());
    }
    staged_changed_path_bytes_from_name_status_z(&output.stdout)
}

#[cfg(test)]
pub(crate) fn staged_changed_paths_from_name_status_z(
    stdout: &[u8],
) -> Result<Vec<String>, String> {
    Ok(staged_changed_path_bytes_from_name_status_z(stdout)?
        .into_iter()
        .map(|path| String::from_utf8_lossy(&path).into_owned())
        .collect())
}

pub(crate) fn staged_changed_path_bytes_from_name_status_z(
    stdout: &[u8],
) -> Result<Vec<Vec<u8>>, String> {
    let mut fields = stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut paths = Vec::new();
    while let Some(status) = fields.next() {
        let Some(path) = fields.next() else {
            return Err("git diff name-status output ended before path".to_string());
        };
        paths.push(path.to_vec());
        if status.starts_with(b"R") || status.starts_with(b"C") {
            let Some(path) = fields.next() else {
                return Err("git diff name-status output ended before rename/copy path".to_string());
            };
            paths.push(path.to_vec());
        }
    }
    Ok(paths)
}

pub(crate) fn is_canon_project_path_bytes(path: &[u8]) -> bool {
    path == b".canon" || path.starts_with(b".canon/")
}

pub(crate) fn is_canon_only_staged_change_bytes(paths: &[Vec<u8>]) -> bool {
    !paths.is_empty() && paths.iter().all(|path| is_canon_project_path_bytes(path))
}
