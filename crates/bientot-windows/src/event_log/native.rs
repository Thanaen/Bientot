//! A query of one event log channel through the Windows Event Log API, ported from the .NET
//! `SystemEventLogSource`.

#![allow(unsafe_code)]

use windows_sys::Win32::Foundation::{
    ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_ITEMS, GetLastError,
};
use windows_sys::Win32::System::EventLog::{
    EVT_HANDLE, EvtClose, EvtNext, EvtQuery, EvtQueryChannelPath, EvtQueryReverseDirection,
    EvtRender, EvtRenderEventXml,
};
use windows_sys::Win32::System::Threading::INFINITE;

use super::{CollectedRecords, LogUnavailable, collect};

/// Up to `record_limit` records of `channel` that match the XPath `query`, newest first.
pub(crate) fn read(
    channel: &str,
    query: &str,
    record_limit: usize,
) -> Result<CollectedRecords, LogUnavailable> {
    let results = run_query(channel, query)?;
    collect(|| next_record(&results), record_limit)
}

struct Handle(EVT_HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from `EvtQuery` or `EvtNext` and is closed once.
        unsafe { EvtClose(self.0) };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn run_query(channel: &str, query: &str) -> Result<Handle, LogUnavailable> {
    let (path, query) = (wide(channel), wide(query));
    // SAFETY: both strings are NUL-terminated and live through the call; a null session queries
    // this computer.
    let results = unsafe {
        EvtQuery(
            0,
            path.as_ptr(),
            query.as_ptr(),
            EvtQueryChannelPath | EvtQueryReverseDirection,
        )
    };
    if results == 0 {
        return Err(LogUnavailable);
    }
    Ok(Handle(results))
}

fn next_record(results: &Handle) -> Result<Option<String>, LogUnavailable> {
    let mut event: EVT_HANDLE = 0;
    let mut returned = 0u32;
    // SAFETY: `event` has room for the one handle asked for and `returned` is a live, writable
    // local.
    if unsafe { EvtNext(results.0, 1, &mut event, INFINITE, 0, &mut returned) } == 0 {
        // SAFETY: reads this thread's last error, set by the call above.
        return match unsafe { GetLastError() } {
            ERROR_NO_MORE_ITEMS => Ok(None),
            _ => Err(LogUnavailable),
        };
    }
    render_xml(&Handle(event)).map(Some)
}

fn render_xml(event: &Handle) -> Result<String, LogUnavailable> {
    let mut buffer = vec![0u16; 4096];
    let mut used_bytes = 0u32;
    let mut property_count = 0u32;
    loop {
        // SAFETY: the buffer is writable for the byte size passed, and both counts are live,
        // writable locals. A null context renders the whole event.
        let rendered = unsafe {
            EvtRender(
                0,
                event.0,
                EvtRenderEventXml,
                (buffer.len() * size_of::<u16>()) as u32,
                buffer.as_mut_ptr().cast(),
                &mut used_bytes,
                &mut property_count,
            )
        };
        if rendered != 0 {
            break;
        }
        let needed = (used_bytes as usize).div_ceil(size_of::<u16>());
        // SAFETY: reads this thread's last error, set by the call above.
        if unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER || needed <= buffer.len() {
            return Err(LogUnavailable);
        }
        buffer.resize(needed, 0);
    }
    let length = (used_bytes as usize / size_of::<u16>()).min(buffer.len());
    let xml = &buffer[..length];
    let xml = xml.strip_suffix(&[0]).unwrap_or(xml);
    Ok(String::from_utf16_lossy(xml))
}
