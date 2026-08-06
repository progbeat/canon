//! Unix temporary-directory parent discovery.

use super::push_unique_path;
use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

pub(super) struct TemporaryParentCandidates {
    memory_backed: Vec<PathBuf>,
    fallback: Vec<PathBuf>,
    mounts: Option<Vec<Mount>>,
}

struct Mount {
    path: PathBuf,
    memory_backed: bool,
    allows_executables: bool,
}

pub(super) fn temporary_parent_candidates() -> TemporaryParentCandidates {
    let mounts = discover_mounts();
    let mut memory_backed = Vec::new();
    add_common_memory_backed_temporary_parent_candidates(&mut memory_backed);
    if let Some(mounts) = &mounts {
        for mount in mounts.iter().filter(|mount| mount.memory_backed) {
            push_unique_path(&mut memory_backed, mount.path.clone());
        }
    }
    let mut fallback = Vec::new();
    push_unique_path(&mut fallback, std::env::temp_dir());
    push_unique_path(&mut fallback, PathBuf::from("/tmp"));
    push_unique_path(&mut fallback, PathBuf::from("/var/tmp"));
    TemporaryParentCandidates {
        memory_backed,
        fallback,
        mounts,
    }
}

impl TemporaryParentCandidates {
    pub(super) fn memory_backed(&self) -> &[PathBuf] {
        &self.memory_backed
    }

    pub(super) fn fallback(&self) -> &[PathBuf] {
        &self.fallback
    }

    pub(super) fn allows_executables(&self, parent: &Path) -> bool {
        let Ok(parent) = parent.canonicalize() else {
            return false;
        };
        self.mounts
            .as_ref()
            .and_then(|mounts| best_matching_mount(mounts, &parent))
            .is_none_or(|mount| mount.allows_executables)
    }
}

fn add_common_memory_backed_temporary_parent_candidates(parents: &mut Vec<PathBuf>) {
    // Prefer common RAM-backed locations before fallback temp directories.
    // Missing paths are skipped later by temporary-directory allocation.
    push_unique_path(parents, PathBuf::from("/dev/shm"));
    push_unique_path(parents, PathBuf::from("/run/shm"));
}

fn discover_mounts() -> Option<Vec<Mount>> {
    mountinfo_mounts().or_else(mounts_mounts)
}

fn mountinfo_mounts() -> Option<Vec<Mount>> {
    read_mounts("/proc/self/mountinfo", |line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let separator = fields.iter().position(|field| *field == "-")?;
        if separator <= 5 || fields.len() <= separator + 3 {
            return None;
        }
        Some(Mount {
            path: proc_mount_path(fields[4]),
            memory_backed: is_memory_backed_filesystem(fields[separator + 1]),
            allows_executables: !has_mount_option(fields[5], "noexec")
                && !has_mount_option(fields[separator + 3], "noexec"),
        })
    })
}

fn mounts_mounts() -> Option<Vec<Mount>> {
    read_mounts("/proc/mounts", |line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        (fields.len() >= 4).then(|| Mount {
            path: proc_mount_path(fields[1]),
            memory_backed: is_memory_backed_filesystem(fields[2]),
            allows_executables: !has_mount_option(fields[3], "noexec"),
        })
    })
}

fn read_mounts(path: &str, parse: impl FnMut(&str) -> Option<Mount>) -> Option<Vec<Mount>> {
    Some(
        fs::read_to_string(path)
            .ok()?
            .lines()
            .filter_map(parse)
            .collect(),
    )
}

fn best_matching_mount<'a>(mounts: &'a [Mount], path: &Path) -> Option<&'a Mount> {
    let mut best_match = None;
    for mount in mounts {
        if path.starts_with(&mount.path) {
            let specificity = mount.path.components().count();
            if best_match
                .as_ref()
                .is_none_or(|(best_specificity, _)| specificity > *best_specificity)
            {
                best_match = Some((specificity, mount));
            }
        }
    }
    best_match.map(|(_, mount)| mount)
}

fn has_mount_option(options: &str, expected: &str) -> bool {
    options.split(',').any(|option| option == expected)
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
