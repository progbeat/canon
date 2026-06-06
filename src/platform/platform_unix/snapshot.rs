use crate::platform::push_unique_path;
use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

pub(crate) fn memory_backed_staged_snapshot_parent_candidates() -> Vec<PathBuf> {
    let mut parents = Vec::new();
    add_common_memory_backed_staged_snapshot_parent_candidates(&mut parents);
    add_discovered_memory_backed_staged_snapshot_parent_candidates(&mut parents);
    parents
}

pub(crate) fn ordinary_staged_snapshot_parent_candidates() -> Vec<PathBuf> {
    vec![std::env::temp_dir()]
}

fn add_common_memory_backed_staged_snapshot_parent_candidates(parents: &mut Vec<PathBuf>) {
    // Prefer common RAM-backed locations before ordinary temp directories.
    // Missing paths are skipped later by snapshot creation.
    push_unique_path(parents, PathBuf::from("/dev/shm"));
    push_unique_path(parents, PathBuf::from("/run/shm"));
}

fn add_discovered_memory_backed_staged_snapshot_parent_candidates(parents: &mut Vec<PathBuf>) {
    // Prefer memory-backed locations when the host exposes them. Missing
    // candidates are harmless: snapshot creation skips paths that do not exist
    // and later falls back to the ordinary temporary directory.
    for path in discover_memory_backed_mount_points() {
        push_unique_path(parents, path);
    }
}

fn discover_memory_backed_mount_points() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    add_mountinfo_memory_backed_paths(&mut paths);
    add_mounts_memory_backed_paths(&mut paths);
    paths
}

fn add_mountinfo_memory_backed_paths(paths: &mut Vec<PathBuf>) {
    let Ok(contents) = fs::read_to_string("/proc/self/mountinfo") else {
        return;
    };
    for line in contents.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let Some(separator) = fields.iter().position(|field| *field == "-") else {
            continue;
        };
        if separator <= 4 || fields.len() <= separator + 1 {
            continue;
        }
        if is_memory_backed_filesystem(fields[separator + 1]) {
            push_unique_path(paths, proc_mount_path(fields[4]));
        }
    }
}

fn add_mounts_memory_backed_paths(paths: &mut Vec<PathBuf>) {
    let Ok(contents) = fs::read_to_string("/proc/mounts") else {
        return;
    };
    for line in contents.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 3 {
            continue;
        }
        if is_memory_backed_filesystem(fields[2]) {
            push_unique_path(paths, proc_mount_path(fields[1]));
        }
    }
}

fn is_memory_backed_filesystem(fs_type: &str) -> bool {
    matches!(fs_type, "tmpfs" | "ramfs")
}

fn proc_mount_path(raw: &str) -> PathBuf {
    let bytes = raw.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' && index + 3 < bytes.len() {
            if let Some(byte) = octal_escape_byte(&bytes[index + 1..index + 4]) {
                output.push(byte);
                index += 4;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    PathBuf::from(OsString::from_vec(output))
}

fn octal_escape_byte(digits: &[u8]) -> Option<u8> {
    if digits.len() != 3 || !digits.iter().all(|digit| matches!(digit, b'0'..=b'7')) {
        return None;
    }
    let value = u16::from(digits[0] - b'0') * 64
        + u16::from(digits[1] - b'0') * 8
        + u16::from(digits[2] - b'0');
    u8::try_from(value).ok()
}
