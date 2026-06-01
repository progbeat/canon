use super::{push_unique_path, wait_for_app_server_child};
use std::ffi::c_void;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::mem;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::ptr;
use std::slice;

#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;

const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

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

pub(crate) fn move_path(source: &Path, target: &Path) -> Result<(), String> {
    fs::rename(source, target).map_err(|err| {
        format!(
            "failed to move isolated path {} to {}: {}",
            source.display(),
            target.display(),
            err
        )
    })
}

pub(crate) fn make_hook_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub(crate) fn set_materialized_permissions(path: &Path) -> Result<(), String> {
    set_readonly(path, true)
}

pub(crate) fn set_private_permissions(path: &Path) -> Result<(), String> {
    set_readonly(path, false)
}

fn set_readonly(path: &Path, readonly: bool) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    let mut permissions = metadata.permissions();
    permissions.set_readonly(readonly);
    fs::set_permissions(path, permissions).map_err(|err| {
        format!(
            "failed to update permissions for {}: {}",
            path.display(),
            err
        )
    })
}

#[derive(Clone)]
pub(crate) struct SecretDirMode {
    permissions: fs::Permissions,
    dacl: Option<Vec<u8>>,
}

pub(crate) fn secret_dir_mode(path: &Path) -> Result<SecretDirMode, String> {
    let metadata = secret_dir_metadata(path)?;
    Ok(SecretDirMode {
        permissions: metadata.permissions(),
        dacl: windows_dacl(path)?,
    })
}

pub(crate) fn chmod_secret_dir_no_access(path: &Path) -> Result<(), String> {
    secret_dir_metadata(path)?;
    windows_set_no_access_dacl(path)
}

pub(crate) fn restore_secret_dir_mode(path: &Path, mode: &SecretDirMode) -> Result<(), String> {
    windows_restore_dacl(path, mode.dacl.as_deref())?;
    fs::set_permissions(path, mode.permissions.clone()).map_err(|err| {
        format!(
            "failed to restore secret dir permissions {}: {}",
            path.display(),
            err
        )
    })
}

fn secret_dir_metadata(path: &Path) -> Result<fs::Metadata, String> {
    let metadata = fs::metadata(path)
        .map_err(|err| format!("failed to stat secret dir {}: {}", path.display(), err))?;
    if !metadata.file_type().is_dir() {
        return Err(format!("secret dir {} is not a directory", path.display()));
    }
    Ok(metadata)
}

pub(crate) fn create_materialized_symlink(target: &[u8], link: &Path) -> Result<(), String> {
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

pub(crate) fn hardlink_file_or_copy_symlink(source: &Path, target: &Path) -> Result<(), String> {
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
    reject_append_symlink(path)?;
    let file = fs::OpenOptions::new()
        .append(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    let metadata = file
        .metadata()
        .map_err(|err| format!("failed to inspect opened {}: {}", path.display(), err))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("refusing to open symlink {}", path.display()));
    }
    if !metadata.file_type().is_file() {
        return Err(format!("refusing to open non-file {}", path.display()));
    }
    Ok(file)
}

fn reject_append_symlink(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("refusing to open symlink {}", path.display()));
    }
    Ok(())
}

pub(crate) fn add_memory_backed_staged_snapshot_parent_candidates(parents: &mut Vec<PathBuf>) {
    add_env_staged_snapshot_parent_candidates(
        parents,
        &["CANON_MEMORY_BACKED_TMPDIR", "RAMDISK", "RAMDISK_TMPDIR"],
    );
}

pub(crate) fn add_ordinary_staged_snapshot_parent_candidates(parents: &mut Vec<PathBuf>) {
    add_env_staged_snapshot_parent_candidates(parents, &["TMPDIR", "TEMP", "TMP"]);
}

fn add_env_staged_snapshot_parent_candidates(parents: &mut Vec<PathBuf>, names: &[&str]) {
    for name in names {
        let Some(value) = std::env::var_os(name) else {
            continue;
        };
        if !value.is_empty() {
            push_unique_path(parents, PathBuf::from(value));
        }
    }
}

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

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

type Dword = u32;
type Bool = i32;
type Psid = *mut c_void;
type SecurityDescriptor = *mut c_void;
type PsecurityDescriptor = *mut c_void;
type Pacl = *mut Acl;

#[repr(C)]
struct Acl {
    acl_revision: u8,
    sbz1: u8,
    acl_size: u16,
    ace_count: u16,
    sbz2: u16,
}

#[repr(C)]
struct AceHeader {
    ace_type: u8,
    ace_flags: u8,
    ace_size: u16,
}

#[repr(C)]
struct AccessDeniedAce {
    header: AceHeader,
    mask: Dword,
    sid_start: Dword,
}

#[repr(C)]
struct AclSizeInformation {
    ace_count: Dword,
    acl_bytes_in_use: Dword,
    acl_bytes_free: Dword,
}

#[repr(C)]
struct SidIdentifierAuthority {
    value: [u8; 6],
}

const ERROR_SUCCESS: Dword = 0;
const SE_FILE_OBJECT: Dword = 1;
const DACL_SECURITY_INFORMATION: Dword = 0x0000_0004;
const ACL_REVISION: Dword = 2;
const ACL_SIZE_INFORMATION_CLASS: Dword = 2;
const OBJECT_INHERIT_ACE: Dword = 0x1;
const CONTAINER_INHERIT_ACE: Dword = 0x2;
const FILE_ALL_ACCESS: Dword = 0x001F_01FF;
const SECURITY_WORLD_RID: Dword = 0;
const SECURITY_WORLD_SID_AUTHORITY: SidIdentifierAuthority = SidIdentifierAuthority {
    value: [0, 0, 0, 0, 0, 1],
};

#[link(name = "advapi32")]
extern "system" {
    fn GetNamedSecurityInfoW(
        object_name: *const u16,
        object_type: Dword,
        security_info: Dword,
        owner: *mut Psid,
        group: *mut Psid,
        dacl: *mut Pacl,
        sacl: *mut Pacl,
        security_descriptor: *mut PsecurityDescriptor,
    ) -> Dword;

    fn SetNamedSecurityInfoW(
        object_name: *mut u16,
        object_type: Dword,
        security_info: Dword,
        owner: Psid,
        group: Psid,
        dacl: Pacl,
        sacl: Pacl,
    ) -> Dword;

    fn GetAclInformation(
        acl: Pacl,
        acl_information: *mut c_void,
        acl_information_length: Dword,
        acl_information_class: Dword,
    ) -> Bool;

    fn InitializeAcl(acl: Pacl, acl_length: Dword, acl_revision: Dword) -> Bool;

    fn AddAccessDeniedAceEx(
        acl: Pacl,
        ace_revision: Dword,
        ace_flags: Dword,
        access_mask: Dword,
        sid: Psid,
    ) -> Bool;

    fn AllocateAndInitializeSid(
        identifier_authority: *const SidIdentifierAuthority,
        sub_authority_count: u8,
        sub_authority0: Dword,
        sub_authority1: Dword,
        sub_authority2: Dword,
        sub_authority3: Dword,
        sub_authority4: Dword,
        sub_authority5: Dword,
        sub_authority6: Dword,
        sub_authority7: Dword,
        sid: *mut Psid,
    ) -> Bool;

    fn FreeSid(sid: Psid) -> *mut c_void;

    fn GetLengthSid(sid: Psid) -> Dword;
}

#[link(name = "kernel32")]
extern "system" {
    fn LocalFree(memory: SecurityDescriptor) -> SecurityDescriptor;
}

struct LocalSecurityDescriptor(SecurityDescriptor);

impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: Windows returns this pointer from GetNamedSecurityInfoW
            // and documents LocalFree as its matching deallocator.
            unsafe {
                let _ = LocalFree(self.0);
            }
        }
    }
}

struct EveryoneSid(Psid);

impl EveryoneSid {
    fn new() -> Result<EveryoneSid, String> {
        let mut sid = ptr::null_mut();
        // SAFETY: All pointers are valid for the call, and the returned SID is
        // owned by this wrapper until FreeSid in Drop.
        let ok = unsafe {
            AllocateAndInitializeSid(
                &SECURITY_WORLD_SID_AUTHORITY,
                1,
                SECURITY_WORLD_RID,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                &mut sid,
            )
        };
        if ok == 0 || sid.is_null() {
            return Err(format!(
                "failed to allocate Everyone SID: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(EveryoneSid(sid))
    }

    fn as_ptr(&self) -> Psid {
        self.0
    }
}

impl Drop for EveryoneSid {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: The SID was allocated by AllocateAndInitializeSid.
            unsafe {
                let _ = FreeSid(self.0);
            }
        }
    }
}

fn windows_dacl(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let path_display = path.display().to_string();
    let path = wide_path(path);
    let mut dacl = ptr::null_mut();
    let mut security_descriptor = ptr::null_mut();
    // SAFETY: The path is a null-terminated UTF-16 string, optional output
    // pointers are null, and `security_descriptor` is freed by the guard below.
    let status = unsafe {
        GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut security_descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(windows_status_error(
            "read secret dir DACL",
            &path_display,
            status,
        ));
    }
    let _security_descriptor = LocalSecurityDescriptor(security_descriptor);
    if dacl.is_null() {
        return Ok(None);
    }
    let mut info = AclSizeInformation {
        ace_count: 0,
        acl_bytes_in_use: 0,
        acl_bytes_free: 0,
    };
    // SAFETY: `dacl` points into the live security descriptor, and `info` is a
    // writable ACL_SIZE_INFORMATION buffer with the expected byte length.
    let ok = unsafe {
        GetAclInformation(
            dacl,
            &mut info as *mut AclSizeInformation as *mut c_void,
            mem::size_of::<AclSizeInformation>() as Dword,
            ACL_SIZE_INFORMATION_CLASS,
        )
    };
    if ok == 0 {
        return Err(format!(
            "failed to inspect secret dir DACL {}: {}",
            path_display,
            io::Error::last_os_error()
        ));
    }
    // SAFETY: GetAclInformation reported the initialized ACL byte length.
    let bytes = unsafe { slice::from_raw_parts(dacl as *const u8, info.acl_bytes_in_use as usize) };
    Ok(Some(bytes.to_vec()))
}

fn windows_set_no_access_dacl(path: &Path) -> Result<(), String> {
    let everyone = EveryoneSid::new()?;
    // SAFETY: The SID pointer is valid for the lifetime of `everyone`.
    let sid_len = unsafe { GetLengthSid(everyone.as_ptr()) as usize };
    let acl_len = mem::size_of::<Acl>() + mem::size_of::<AccessDeniedAce>()
        - mem::size_of::<Dword>()
        + sid_len;
    let mut acl_storage = vec![0_u8; acl_len];
    let acl = acl_storage.as_mut_ptr() as Pacl;
    // SAFETY: `acl_storage` is a writable buffer of `acl_len` bytes.
    if unsafe { InitializeAcl(acl, acl_len as Dword, ACL_REVISION) } == 0 {
        return Err(format!(
            "failed to initialize no-access secret dir DACL: {}",
            io::Error::last_os_error()
        ));
    }
    // SAFETY: `acl` is initialized and `everyone` is a valid SID.
    if unsafe {
        AddAccessDeniedAceEx(
            acl,
            ACL_REVISION,
            OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
            FILE_ALL_ACCESS,
            everyone.as_ptr(),
        )
    } == 0
    {
        return Err(format!(
            "failed to populate no-access secret dir DACL: {}",
            io::Error::last_os_error()
        ));
    }
    windows_set_dacl(path, acl, "chmod secret dir")
}

fn windows_restore_dacl(path: &Path, dacl: Option<&[u8]>) -> Result<(), String> {
    let mut dacl_storage = dacl.map(|bytes| bytes.to_vec());
    let dacl = dacl_storage
        .as_mut()
        .map_or(ptr::null_mut(), |bytes| bytes.as_mut_ptr() as Pacl);
    windows_set_dacl(path, dacl, "restore secret dir DACL")
}

fn windows_set_dacl(path: &Path, dacl: Pacl, action: &str) -> Result<(), String> {
    let path_display = path.display().to_string();
    let mut path = wide_path(path);
    // SAFETY: The path is a null-terminated UTF-16 string. The DACL either
    // points to a live ACL buffer for the call duration or is null to restore a
    // null DACL captured from the original descriptor.
    let status = unsafe {
        SetNamedSecurityInfoW(
            path.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            dacl,
            ptr::null_mut(),
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(windows_status_error(action, &path_display, status))
    }
}

fn windows_status_error(action: &str, path: &str, status: Dword) -> String {
    format!(
        "failed to {} {}: {}",
        action,
        path,
        io::Error::from_raw_os_error(status as i32)
    )
}

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
