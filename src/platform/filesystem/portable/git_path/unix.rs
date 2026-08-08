use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

pub(super) fn remove_git_stdout_record_terminator(bytes: &mut Vec<u8>) -> Result<(), String> {
    if bytes.last() != Some(&b'\n') {
        return Err("Git path output must end with a line-feed record terminator".to_string());
    }
    bytes.truncate(bytes.len() - 1);
    Ok(())
}

pub(super) fn path_from_git_bytes(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(OsString::from_vec(bytes))
}

pub(super) fn git_path_bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().as_bytes().to_vec()
}

pub(super) fn os_string_from_bytes(bytes: Vec<u8>) -> OsString {
    OsString::from_vec(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] // xpec: gO
    fn git_path_output_removes_only_its_record_terminator() {
        let mut output = b"/repo\r\n\n".to_vec();
        remove_git_stdout_record_terminator(&mut output).unwrap();
        assert_eq!(output, b"/repo\r\n");
    }

    #[test] // xpec: gO
    fn git_path_output_without_a_record_terminator_is_rejected() {
        let mut output = b"/repo".to_vec();
        assert!(remove_git_stdout_record_terminator(&mut output).is_err());
    }
}
