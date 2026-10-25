//! The registry calls of the .NET `RegistryStartupStore`.

#![allow(unsafe_code)]

use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS, ERROR_UNSUPPORTED_TYPE,
};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_BINARY, REG_VALUE_TYPE, RRF_NOEXPAND, RRF_RT_ANY, RRF_RT_REG_EXPAND_SZ,
    RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
};

use super::{ApprovalValue, RunKey, StartupApproval, decode_approval};
use crate::registry::{self, check, os_error, wide};

const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_APPROVED_RUN_KEY_PATH: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

/// `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`.
pub struct RegistryRunKey;

impl RunKey for RegistryRunKey {
    fn read_run_value(&self, name: &str) -> io::Result<Option<String>> {
        // .NET reads strings unexpanded and any other type as missing.
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
        let Some((_, data)) = get_value(RUN_KEY_PATH, name, flags)? else {
            return Ok(None);
        };
        let units: Vec<u16> = data
            .chunks_exact(2)
            .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
            .collect();
        let text = units.split(|&unit| unit == 0).next().unwrap_or_default();
        Ok(Some(String::from_utf16_lossy(text)))
    }

    /// Creates the key when it is missing, as .NET's `CreateSubKey`.
    fn set_run_value(&self, name: &str, command: &str) -> io::Result<()> {
        registry::set_string(RUN_KEY_PATH, name, command.as_ref())
    }

    fn delete_run_value(&self, name: &str) -> io::Result<()> {
        let subkey = wide(RUN_KEY_PATH);
        let name = wide(name);
        // SAFETY: both strings are NUL-terminated UTF-16 borrowed for the call.
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey.as_ptr(), name.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => Ok(()),
            status => check(status),
        }
    }

    fn read_approval(&self, name: &str) -> io::Result<StartupApproval> {
        let value =
            get_value(STARTUP_APPROVED_RUN_KEY_PATH, name, RRF_RT_ANY)?.map(|(kind, data)| {
                match kind {
                    REG_BINARY => ApprovalValue::Binary(data),
                    _ => ApprovalValue::OtherType,
                }
            });
        Ok(decode_approval(value))
    }
}

/// The type and bytes of `subkey\name` under `HKEY_CURRENT_USER`, or `None` when the key or
/// the value is missing or its type is not one `flags` accepts.
fn get_value(
    subkey: &str,
    name: &str,
    flags: u32,
) -> io::Result<Option<(REG_VALUE_TYPE, Vec<u8>)>> {
    let subkey = wide(subkey);
    let name = wide(name);
    let mut data = Vec::<u8>::new();
    loop {
        let mut kind: REG_VALUE_TYPE = 0;
        let mut size = u32::try_from(data.len()).unwrap_or(u32::MAX);
        let buffer = if data.is_empty() {
            ptr::null_mut()
        } else {
            data.as_mut_ptr().cast()
        };
        // SAFETY: the key path and value name are NUL-terminated UTF-16 strings borrowed for the
        // call. `buffer` is null, to ask for the size only, or points to `size` writable bytes of
        // `data`; the call writes at most `size` bytes and stores the length it needs in `size`.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                name.as_ptr(),
                flags,
                &mut kind,
                buffer,
                &mut size,
            )
        };
        match status {
            ERROR_FILE_NOT_FOUND | ERROR_UNSUPPORTED_TYPE => return Ok(None),
            // The value changed between the calls: ask again with the new size.
            ERROR_MORE_DATA => data.resize(size as usize, 0),
            ERROR_SUCCESS if buffer.is_null() && size > 0 => data.resize(size as usize, 0),
            ERROR_SUCCESS => {
                data.truncate(size as usize);
                return Ok(Some((kind, data)));
            }
            status => return Err(os_error(status)),
        }
    }
}
