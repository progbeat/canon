use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};

pub(crate) fn path_from_git_bytes(bytes: Vec<u8>) -> Result<PathBuf, String> {
    git_bytes_os_string(bytes).map(PathBuf::from)
}

pub(crate) fn git_path_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    let mut index = 0;
    while index < units.len() {
        let unit = units[index];
        if unit == 0 {
            return Err(format!("git path must not contain NUL: {}", path.display()));
        } else if is_windows_separator(unit) {
            bytes.push(b'/');
            index += 1;
        } else if let Some(byte) = surrogate_escaped_byte(unit) {
            if byte == 0 {
                return Err(format!("git path must not contain NUL: {}", path.display()));
            }
            bytes.push(byte);
            index += 1;
        } else if is_high_surrogate(unit) {
            let Some(&low) = units.get(index + 1) else {
                return Err(format!(
                    "git path contains unpaired surrogate: {}",
                    path.display()
                ));
            };
            bytes.extend(
                utf16_surrogate_pair_to_char(unit, low, path)?
                    .encode_utf8(&mut [0; 4])
                    .as_bytes(),
            );
            index += 2;
        } else if is_low_surrogate(unit) {
            return Err(format!(
                "git path contains unpaired surrogate: {}",
                path.display()
            ));
        } else {
            bytes.extend(
                utf16_unit_to_char(unit, path)?
                    .encode_utf8(&mut [0; 4])
                    .as_bytes(),
            );
            index += 1;
        }
    }
    Ok(bytes)
}

pub(crate) fn os_string_from_bytes(bytes: Vec<u8>) -> Result<OsString, String> {
    git_bytes_os_string(bytes)
}

pub(super) fn git_bytes_os_string(bytes: Vec<u8>) -> Result<OsString, String> {
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

fn is_windows_separator(unit: u16) -> bool {
    unit == b'/' as u16 || unit == b'\\' as u16
}

fn surrogate_escaped_byte(unit: u16) -> Option<u8> {
    (0xDC00..=0xDCFF)
        .contains(&unit)
        .then_some((unit & 0x00FF) as u8)
}

fn is_high_surrogate(unit: u16) -> bool {
    (0xD800..=0xDBFF).contains(&unit)
}

fn is_low_surrogate(unit: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&unit)
}

fn utf16_surrogate_pair_to_char(high: u16, low: u16, path: &Path) -> Result<char, String> {
    if !is_low_surrogate(low) {
        return Err(format!(
            "git path contains unpaired surrogate: {}",
            path.display()
        ));
    }
    let codepoint = 0x1_0000 + ((((high - 0xD800) as u32) << 10) | ((low - 0xDC00) as u32));
    char::from_u32(codepoint).ok_or_else(|| {
        format!(
            "git path contains invalid UTF-16 scalar: {}",
            path.display()
        )
    })
}

fn utf16_unit_to_char(unit: u16, path: &Path) -> Result<char, String> {
    char::from_u32(unit as u32).ok_or_else(|| {
        format!(
            "git path contains invalid UTF-16 scalar: {}",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_path_bytes_round_trips_surrogate_escaped_git_bytes() {
        let original = b"dir/\xFF/name with spaces/\x80.bin".to_vec();
        let path = path_from_git_bytes(original.clone()).unwrap();
        assert_eq!(git_path_bytes(&path).unwrap(), original);
    }

    #[test]
    fn git_path_bytes_uses_git_separators() {
        let path = PathBuf::from(r"dir\file.txt");
        assert_eq!(git_path_bytes(&path).unwrap(), b"dir/file.txt");
    }
}
