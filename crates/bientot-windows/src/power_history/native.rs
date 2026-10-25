//! The System log query of the .NET `SystemEventLogSource`, through the Windows Event Log API.

#![allow(unsafe_code)]

use windows_sys::Win32::Foundation::{
    ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_ITEMS, GetLastError,
};
use windows_sys::Win32::System::EventLog::{
    EVT_HANDLE, EvtClose, EvtNext, EvtQuery, EvtQueryChannelPath, EvtQueryReverseDirection,
    EvtRender, EvtRenderEventXml,
};
use windows_sys::Win32::System::Threading::INFINITE;

use super::{CollectedRecords, SystemEventLog, SystemLogUnavailable, collect};

const QUERY: &str = "*[System[((Provider[@Name='Microsoft-Windows-Kernel-General']) and (EventID=12 or EventID=13)) or ((Provider[@Name='Microsoft-Windows-Kernel-Power']) and (EventID=42 or EventID=107))]]";

/// The `System` log of this computer, read newest first.
pub struct SystemLog;

impl SystemEventLog for SystemLog {
    fn read(&self, record_limit: usize) -> Result<CollectedRecords, SystemLogUnavailable> {
        let results = query()?;
        collect(|| next_record(&results), record_limit)
    }
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

fn query() -> Result<Handle, SystemLogUnavailable> {
    let (path, query) = (wide("System"), wide(QUERY));
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
        return Err(SystemLogUnavailable);
    }
    Ok(Handle(results))
}

fn next_record(results: &Handle) -> Result<Option<String>, SystemLogUnavailable> {
    let mut event: EVT_HANDLE = 0;
    let mut returned = 0u32;
    // SAFETY: `event` has room for the one handle asked for and `returned` is a live, writable
    // local.
    if unsafe { EvtNext(results.0, 1, &mut event, INFINITE, 0, &mut returned) } == 0 {
        // SAFETY: reads this thread's last error, set by the call above.
        return match unsafe { GetLastError() } {
            ERROR_NO_MORE_ITEMS => Ok(None),
            _ => Err(SystemLogUnavailable),
        };
    }
    render_xml(&Handle(event)).map(Some)
}

fn render_xml(event: &Handle) -> Result<String, SystemLogUnavailable> {
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
            return Err(SystemLogUnavailable);
        }
        buffer.resize(needed, 0);
    }
    let length = (used_bytes as usize / size_of::<u16>()).min(buffer.len());
    let xml = &buffer[..length];
    let xml = xml.strip_suffix(&[0]).unwrap_or(xml);
    Ok(String::from_utf16_lossy(xml))
}

#[cfg(test)]
mod tests {
    use bientot_domain::intervals::UtcInterval;
    use jiff::Timestamp;

    use super::*;
    use crate::power_history::{PowerHistoryRead, PowerHistoryReader, RECORD_LIMIT};

    #[test]
    fn the_system_log_reads_and_every_matching_record_parses() {
        let read = PowerHistoryReader::new(SystemLog, RECORD_LIMIT)
            .read(UtcInterval::new(Timestamp::UNIX_EPOCH, Timestamp::now()).unwrap());

        assert!(
            matches!(
                read,
                PowerHistoryRead::Complete(_) | PowerHistoryRead::Truncated
            ),
            "{read:?}"
        );
    }
}
