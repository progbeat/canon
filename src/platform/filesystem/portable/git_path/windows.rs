use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::os::windows::ffi::OsStringExt;

pub(in super::super) fn remove_git_stdout_record_terminator(
    bytes: &mut Vec<u8>,
) -> Result<(), String> {
    let terminator_len = if bytes.ends_with(b"\r\n") {
        2
    } else if bytes.ends_with(b"\n") {
        1
    } else {
        return Err("Git path output must end with a line terminator".to_string());
    };
    bytes.truncate(bytes.len() - terminator_len);
    Ok(())
}

pub(in super::super) fn path_from_git_bytes(bytes: Vec<u8>) -> Result<PathBuf, String> {
    git_bytes_os_string(bytes).map(PathBuf::from)
}

pub(in super::super) fn git_path_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let path = path.to_str().ok_or_else(|| {
        format!(
            "git path must contain only valid Unicode scalar values: {}",
            path.display()
        )
    })?;
    if path.contains('\0') {
        return Err(format!("git path must not contain NUL: {}", path));
    }
    Ok(path.replace('\\', "/").into_bytes())
}

pub(in super::super) fn os_string_from_bytes(bytes: Vec<u8>) -> Result<OsString, String> {
    git_bytes_os_string(bytes)
}

pub(in super::super) fn git_bytes_os_string(bytes: Vec<u8>) -> Result<OsString, String> {
    if bytes.contains(&0) {
        return Err("Git paths must not contain NUL bytes".to_string());
    }
    String::from_utf8(bytes)
        .map(OsString::from)
        .map_err(|err| format!("Git path must be valid UTF-8: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: gO
    fn git_path_output_removes_one_native_record_terminator() {
        let mut output = b"C:/repo\r\n".to_vec();
        remove_git_stdout_record_terminator(&mut output).unwrap();
        assert_eq!(output, b"C:/repo");

        let mut output = b"C:/repo\n".to_vec();
        remove_git_stdout_record_terminator(&mut output).unwrap();
        assert_eq!(output, b"C:/repo");
    }

    #[test] // xpec: gO
    fn git_path_output_without_a_record_terminator_is_rejected() {
        let mut output = b"C:/repo".to_vec();
        assert!(remove_git_stdout_record_terminator(&mut output).is_err());
    }

    #[test] // xpec: 1g,gO
    fn git_path_conversions_reject_unrepresentable_values() {
        assert!(path_from_git_bytes(b"dir/\xFF".to_vec()).is_err());
        let lone_surrogate = OsString::from_wide(&[0xDC80]);
        assert!(git_path_bytes(Path::new(&lone_surrogate)).is_err());
    }

    #[test] // xpec: 1g
    fn git_path_bytes_uses_git_separators() {
        let path = PathBuf::from(r"dir\file.txt");
        assert_eq!(git_path_bytes(&path).unwrap(), b"dir/file.txt");
    }
}
