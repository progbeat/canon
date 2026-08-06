//! Windows filesystem security primitives shared by portable components.

use std::ffi::c_void;
use std::io;
use std::mem;
use std::path::Path;
use std::ptr;
use std::slice;

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

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

#[derive(Clone)]
pub(super) struct WindowsDacl {
    // WinAPI consumes the complete allocation declared by ACL::AclSize, while
    // ACE-list operations consume only AclBytesInUse from that allocation.
    storage: Vec<u8>,
    bytes_in_use: usize,
}

#[repr(C)]
struct SidIdentifierAuthority {
    value: [u8; 6],
}

const ERROR_SUCCESS: Dword = 0;
const SE_FILE_OBJECT: Dword = 1;
const DACL_SECURITY_INFORMATION: Dword = 0x0000_0004;
const UNPROTECTED_DACL_SECURITY_INFORMATION: Dword = 0x2000_0000;
const ACL_REVISION: Dword = 2;
const ACL_SIZE_INFORMATION_CLASS: Dword = 2;
const MAXDWORD: Dword = Dword::MAX;
const OBJECT_INHERIT_ACE: Dword = 0x1;
const CONTAINER_INHERIT_ACE: Dword = 0x2;
const FILE_WRITE_DATA: Dword = 0x0000_0002;
const FILE_APPEND_DATA: Dword = 0x0000_0004;
const FILE_WRITE_EA: Dword = 0x0000_0010;
const FILE_DELETE_CHILD: Dword = 0x0000_0040;
const FILE_WRITE_ATTRIBUTES: Dword = 0x0000_0100;
const DELETE: Dword = 0x0001_0000;
const FILE_ALL_ACCESS: Dword = 0x001F_01FF;
const MATERIALIZED_WRITE_ACCESS: Dword = FILE_WRITE_DATA
    | FILE_APPEND_DATA
    | FILE_WRITE_EA
    | FILE_DELETE_CHILD
    | FILE_WRITE_ATTRIBUTES
    | DELETE;
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

    fn AddAccessAllowedAceEx(
        acl: Pacl,
        ace_revision: Dword,
        ace_flags: Dword,
        access_mask: Dword,
        sid: Psid,
    ) -> Bool;

    fn AddAce(
        acl: Pacl,
        ace_revision: Dword,
        starting_ace_index: Dword,
        ace_list: *const c_void,
        ace_list_length: Dword,
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

pub(super) fn windows_read_dacl(path: &Path) -> Result<Option<WindowsDacl>, String> {
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
            "read Windows DACL",
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
            "failed to inspect Windows DACL {}: {}",
            path_display,
            io::Error::last_os_error()
        ));
    }
    let bytes_in_use = info.acl_bytes_in_use as usize;
    let allocation_size = bytes_in_use
        .checked_add(info.acl_bytes_free as usize)
        .ok_or_else(|| format!("Windows DACL size overflow for {}", path_display))?;
    // SAFETY: `dacl` points to the ACL inside the live security descriptor.
    let declared_size = unsafe { (*dacl).acl_size as usize };
    if bytes_in_use < mem::size_of::<Acl>()
        || allocation_size < bytes_in_use
        || allocation_size != declared_size
    {
        return Err(format!("invalid Windows DACL size for {}", path_display));
    }
    // SAFETY: GetAclInformation reports the initialized bytes and unused
    // capacity that together form the allocation declared by the ACL header.
    let storage = unsafe { slice::from_raw_parts(dacl as *const u8, allocation_size) }.to_vec();
    Ok(Some(WindowsDacl {
        storage,
        bytes_in_use,
    }))
}

pub(super) fn windows_set_no_access_dacl(path: &Path) -> Result<(), String> {
    windows_set_deny_dacl(
        path,
        FILE_ALL_ACCESS,
        OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
        "chmod secret dir",
    )
}

pub(super) fn windows_set_materialized_readonly_dacl(
    path: &Path,
    is_dir: bool,
) -> Result<(), String> {
    let inherit_flags = if is_dir {
        OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE
    } else {
        0
    };
    let existing = windows_read_dacl(path)?;
    let mut dacl =
        windows_dacl_with_deny(existing.as_ref(), MATERIALIZED_WRITE_ACCESS, inherit_flags)?;
    windows_set_dacl(
        path,
        dacl.as_mut_ptr() as Pacl,
        "set materialized read-only DACL",
    )
}

pub(super) fn windows_reset_dacl_to_inherited(path: &Path) -> Result<(), String> {
    // Let Windows derive the target DACL from its parent. The inheritance
    // engine filters ACEs for the child object type and marks them inherited;
    // directly copying the parent's ACL would not preserve those semantics.
    windows_set_dacl_with_security_info(
        path,
        ptr::null_mut(),
        DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION,
        "restore inherited Windows DACL",
    )
}

fn windows_dacl_with_deny(
    existing: Option<&WindowsDacl>,
    access_mask: Dword,
    ace_flags: Dword,
) -> Result<Vec<u8>, String> {
    let everyone = EveryoneSid::new()?;
    // SAFETY: `everyone` owns a valid SID for this function's duration.
    let sid_len = unsafe { GetLengthSid(everyone.as_ptr()) as usize };
    let ace_len = mem::size_of::<AccessDeniedAce>() - mem::size_of::<Dword>() + sid_len;
    let existing_ace_bytes = existing.map_or(0, |dacl| {
        dacl.bytes_in_use
            .checked_sub(mem::size_of::<Acl>())
            .expect("windows_read_dacl always returns an ACL header")
    });
    let allow_len = if existing.is_none() { ace_len } else { 0 };
    let acl_len = mem::size_of::<Acl>()
        .checked_add(ace_len)
        .and_then(|len| len.checked_add(existing_ace_bytes))
        .and_then(|len| len.checked_add(allow_len))
        .ok_or_else(|| "materialized Windows DACL length overflowed".to_string())?;
    let acl_len_dword = Dword::try_from(acl_len)
        .map_err(|_| "materialized Windows DACL is too large".to_string())?;
    let mut storage = vec![0_u8; acl_len];
    let acl = storage.as_mut_ptr() as Pacl;
    // SAFETY: `storage` is a writable buffer of `acl_len` bytes.
    if unsafe { InitializeAcl(acl, acl_len_dword, ACL_REVISION) } == 0 {
        return Err(format!(
            "failed to initialize materialized Windows DACL: {}",
            io::Error::last_os_error()
        ));
    }
    // SAFETY: `acl` is initialized and `everyone` remains valid.
    if unsafe { AddAccessDeniedAceEx(acl, ACL_REVISION, ace_flags, access_mask, everyone.as_ptr()) }
        == 0
    {
        return Err(format!(
            "failed to add materialized Windows deny ACE: {}",
            io::Error::last_os_error()
        ));
    }
    if let Some(existing) = existing {
        if existing_ace_bytes != 0 {
            let existing_ace_bytes = Dword::try_from(existing_ace_bytes)
                .map_err(|_| "existing Windows DACL is too large".to_string())?;
            // SAFETY: `existing` contains a copied ACL. Bytes after its header
            // are the complete ACE list reported by GetAclInformation.
            let ok = unsafe {
                AddAce(
                    acl,
                    ACL_REVISION,
                    MAXDWORD,
                    existing.storage.as_ptr().add(mem::size_of::<Acl>()) as *const c_void,
                    existing_ace_bytes,
                )
            };
            if ok == 0 {
                return Err(format!(
                    "failed to preserve existing Windows DACL entries: {}",
                    io::Error::last_os_error()
                ));
            }
        }
    } else {
        // A null DACL grants full access. Preserve its read behavior while the
        // preceding deny ACE removes only materialized write capabilities.
        // SAFETY: `acl` is initialized and has capacity for this second ACE.
        if unsafe {
            AddAccessAllowedAceEx(
                acl,
                ACL_REVISION,
                ace_flags,
                FILE_ALL_ACCESS,
                everyone.as_ptr(),
            )
        } == 0
        {
            return Err(format!(
                "failed to preserve null Windows DACL access: {}",
                io::Error::last_os_error()
            ));
        }
    }
    Ok(storage)
}

fn windows_set_deny_dacl(
    path: &Path,
    access_mask: Dword,
    ace_flags: Dword,
    action: &str,
) -> Result<(), String> {
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
    if unsafe { AddAccessDeniedAceEx(acl, ACL_REVISION, ace_flags, access_mask, everyone.as_ptr()) }
        == 0
    {
        return Err(format!(
            "failed to populate Windows deny DACL: {}",
            io::Error::last_os_error()
        ));
    }
    windows_set_dacl(path, acl, action)
}

pub(super) fn windows_apply_dacl(path: &Path, dacl: Option<&WindowsDacl>) -> Result<(), String> {
    let mut dacl_storage = dacl.map(|dacl| dacl.storage.clone());
    let dacl = dacl_storage
        .as_mut()
        .map_or(ptr::null_mut(), |bytes| bytes.as_mut_ptr() as Pacl);
    windows_set_dacl(path, dacl, "restore Windows DACL")
}

fn windows_set_dacl(path: &Path, dacl: Pacl, action: &str) -> Result<(), String> {
    windows_set_dacl_with_security_info(path, dacl, DACL_SECURITY_INFORMATION, action)
}

fn windows_set_dacl_with_security_info(
    path: &Path,
    dacl: Pacl,
    security_information: Dword,
    action: &str,
) -> Result<(), String> {
    let path_display = path.display().to_string();
    let mut path = wide_path(path);
    // SAFETY: The path is a null-terminated UTF-16 string. The DACL either
    // points to a live ACL buffer for the call duration or is null with flags
    // that select null-DACL restoration or Windows-managed ACL inheritance.
    let status = unsafe {
        SetNamedSecurityInfoW(
            path.as_mut_ptr(),
            SE_FILE_OBJECT,
            security_information,
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

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
