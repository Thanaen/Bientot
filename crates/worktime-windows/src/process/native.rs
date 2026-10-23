//! The Win32 calls behind the wait.

#![allow(unsafe_code)]

use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};

/// Waits up to `timeout` for the process `id` to end. Whether it ended: a process that cannot be
/// opened has already ended.
pub fn wait_for_exit(id: u32, timeout: Duration) -> bool {
    // SAFETY: plain call; a null handle means no such process.
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, id) };
    if process.is_null() {
        return true;
    }
    let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
    // SAFETY: `process` is a handle this function opened with the right to wait on it.
    let waited = unsafe { WaitForSingleObject(process, milliseconds) };
    // SAFETY: closed once, after its last use.
    unsafe { CloseHandle(process) };
    waited == WAIT_OBJECT_0
}
