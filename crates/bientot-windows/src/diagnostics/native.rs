//! The Win32 calls behind the diagnostic export.

#![allow(unsafe_code)]

use std::fs::File;
use std::iter;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::Path;
use std::ptr;

use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_NAME_NORMALIZED, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, GetFinalPathNameByHandleW, OPEN_EXISTING,
};
use windows_sys::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE_UNKNOWN, OSVERSIONINFOEXW, OSVERSIONINFOW,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};

use super::{is_directly_inside, names_directory, without_verbatim_prefix};

/// Whether `directory` is its own final path: no link or mount point leads elsewhere.
pub fn directory_is_final(directory: &Path) -> bool {
    let name: Vec<u16> = directory
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    // SAFETY: `name` is NUL-terminated and outlives the call. No access is asked: the handle
    // only names the directory, which `FILE_FLAG_BACKUP_SEMANTICS` lets it open.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return false;
    }
    // SAFETY: a valid handle this function owns, closed when dropped.
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    final_path(handle.as_raw_handle())
        .is_some_and(|path| names_directory(&path, &directory.to_string_lossy()))
}

/// Whether the opened `file` really lies directly inside `directory`.
pub fn file_is_in(file: &File, directory: &Path) -> bool {
    final_path(file.as_raw_handle())
        .is_some_and(|path| is_directly_inside(&path, &directory.to_string_lossy()))
}

/// The final path of `handle` in the form .NET compares: no verbatim prefix, made full.
fn final_path(handle: HANDLE) -> Option<String> {
    let mut buffer = vec![0_u16; 512];
    // .NET asks again once with the length the first call answers.
    for _ in 0..2 {
        let capacity = u32::try_from(buffer.len()).ok()?;
        // SAFETY: `handle` is open and `buffer` holds `capacity` units.
        let length = unsafe {
            GetFinalPathNameByHandleW(handle, buffer.as_mut_ptr(), capacity, FILE_NAME_NORMALIZED)
        } as usize;
        if length == 0 {
            return None;
        }
        if length < buffer.len() {
            let path = without_verbatim_prefix(&String::from_utf16_lossy(&buffer[..length]));
            return std::path::absolute(path)
                .ok()
                .map(|path| path.to_string_lossy().into_owned());
        }
        buffer.resize(length + 1, 0);
    }
    None
}

/// The Windows version as `Environment.OSVersion` reads it: from `RtlGetVersion`, which no
/// compatibility shim lowers, with the service pack as the revision.
pub fn operating_system_version() -> [u32; 4] {
    let mut version = OSVERSIONINFOEXW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOEXW>() as u32,
        ..Default::default()
    };
    // SAFETY: `version` is an `OSVERSIONINFOEXW` whose size field says so, as the call accepts.
    let status = unsafe { RtlGetVersion((&raw mut version).cast::<OSVERSIONINFOW>()) };
    if status < 0 {
        return [0; 4];
    }
    [
        version.dwMajorVersion,
        version.dwMinorVersion,
        version.dwBuildNumber,
        (u32::from(version.wServicePackMajor) << 16) | u32::from(version.wServicePackMinor),
    ]
}

/// The `IMAGE_FILE_MACHINE` of the processor Windows runs on, even under emulation, as
/// `RuntimeInformation.OSArchitecture` reads it. `None` when Windows does not say.
pub fn native_machine() -> Option<u16> {
    let (mut process, mut native) = (IMAGE_FILE_MACHINE_UNKNOWN, IMAGE_FILE_MACHINE_UNKNOWN);
    // SAFETY: the pseudo handle of this process needs no closing; both out pointers are valid.
    let answered = unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, &mut native) };
    (answered != 0 && native != IMAGE_FILE_MACHINE_UNKNOWN).then_some(native)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn a_plain_directory_and_a_file_in_it_are_final() {
        let root = tempfile::tempdir().unwrap();
        // The temporary folder may be named by its 8.3 short name, which is not final.
        let directory = PathBuf::from(without_verbatim_prefix(
            &fs::canonicalize(root.path()).unwrap().to_string_lossy(),
        ));
        let path = directory.join("worktime.jsonl");
        fs::write(&path, b"{}").unwrap();

        assert!(directory_is_final(&directory));
        assert!(file_is_in(&File::open(&path).unwrap(), &directory));
        assert!(!file_is_in(
            &File::open(&path).unwrap(),
            directory.parent().unwrap()
        ));
    }
}
