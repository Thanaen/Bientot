//! The `HKEY_CURRENT_USER` string writes of the start-with-Windows and toast registrations.

#![allow(unsafe_code)]

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt as _;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RegDeleteTreeW, RegSetKeyValueW,
};

/// Writes `data` as the string value `name` of `subkey`, creating the missing keys of the path.
/// An empty `name` is the key's default value.
pub(crate) fn set_string(subkey: &str, name: &str, data: &OsStr) -> io::Result<()> {
    let data = wide(data);
    let size = u32::try_from(data.len() * 2)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "the value is too long"))?;
    let subkey = wide(subkey);
    let name = wide(name);
    // SAFETY: the key path and value name are NUL-terminated UTF-16 strings, and `data` holds
    // `size` readable bytes, the value and its terminator, all borrowed for the call.
    check(unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            name.as_ptr(),
            REG_SZ,
            data.as_ptr().cast(),
            size,
        )
    })
}

/// Deletes `subkey` and everything under it. A missing key is already deleted.
pub(crate) fn delete_tree(subkey: &str) -> io::Result<()> {
    let subkey = wide(subkey);
    // SAFETY: a NUL-terminated UTF-16 path borrowed for the call.
    match unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, subkey.as_ptr()) } {
        ERROR_FILE_NOT_FOUND => Ok(()),
        status => check(status),
    }
}

/// `text` as a NUL-terminated UTF-16 string.
pub(crate) fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain([0]).collect()
}

pub(crate) fn check(status: WIN32_ERROR) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(os_error(status))
    }
}

pub(crate) fn os_error(status: WIN32_ERROR) -> io::Error {
    io::Error::from_raw_os_error(status as i32)
}
