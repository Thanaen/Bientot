//! The Winlogon log query and the security identifier of the user running the app.

#![allow(unsafe_code)]

use jiff::Timestamp;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::event_log::{CollectedRecords, LogUnavailable, native};

use super::WinlogonEventLog;
use super::event_xml::NOTIFICATION_STARTED;

const CHANNEL: &str = "Microsoft-Windows-Winlogon/Operational";

/// The Winlogon log of this computer, read newest first, for one user.
pub struct WinlogonLog {
    user_id: String,
}

impl WinlogonLog {
    /// `None` unless `user_id` is spelled as a security identifier, so that it can stand in the
    /// query as it is.
    pub fn new(user_id: &str) -> Option<Self> {
        let valid = user_id.starts_with("S-")
            && user_id[2..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'-');
        valid.then(|| Self {
            user_id: user_id.to_owned(),
        })
    }
}

impl WinlogonEventLog for WinlogonLog {
    fn user_id(&self) -> &str {
        &self.user_id
    }

    fn read(
        &self,
        since: Timestamp,
        record_limit: usize,
    ) -> Result<CollectedRecords, LogUnavailable> {
        let since = since.strftime("%Y-%m-%dT%H:%M:%S%.3fZ");
        let query = format!(
            "*[System[Provider[@Name='Microsoft-Windows-Winlogon'] and EventID={NOTIFICATION_STARTED} and Security[@UserID='{}'] and TimeCreated[@SystemTime>='{since}']]]",
            self.user_id
        );
        native::read(CHANNEL, &query, record_limit)
    }
}

/// The security identifier of the user running this process, as `S-1-5-21-…`.
pub fn current_user_id() -> Option<String> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo handle of the current process needs no closing, and `token` is a live,
    // writable local.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    let user_id = token_user_id(token);
    // SAFETY: `token` was opened above and is closed once.
    unsafe { CloseHandle(token) };
    user_id
}

fn token_user_id(token: HANDLE) -> Option<String> {
    let mut needed = 0u32;
    // SAFETY: a null buffer of size 0 only asks for the size, written to a live local.
    unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        return None;
    }
    // `u64` words keep the buffer aligned for `TOKEN_USER`.
    let mut buffer = vec![0u64; (needed as usize).div_ceil(size_of::<u64>())];
    // SAFETY: the buffer is writable for at least `needed` bytes.
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return None;
    }
    // SAFETY: the call above filled the buffer with a `TOKEN_USER`, whose SID points into it.
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: `sid` is valid while `buffer` lives, and `text` is a live, writable local.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return None;
    }
    // SAFETY: on success `text` is a NUL-terminated string the system allocated.
    let length = (0..).take_while(|&i| unsafe { *text.add(i) } != 0).count();
    // SAFETY: `text` holds `length` characters before its NUL.
    let user_id = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) }).ok();
    // SAFETY: `text` was allocated by `ConvertSidToStringSidW` and is freed once.
    unsafe { LocalFree(text.cast()) };
    user_id
}

#[cfg(test)]
mod tests {
    use bientot_domain::intervals::UtcInterval;

    use super::*;
    use crate::session_history::{RECORD_LIMIT, SessionHistoryRead, SessionHistoryReader};

    #[test]
    fn the_winlogon_log_of_the_current_user_reads_and_every_matching_record_parses() {
        let user_id = current_user_id().expect("the process has a user");
        let log = WinlogonLog::new(&user_id).expect("a security identifier");
        let now = Timestamp::now();
        let read = SessionHistoryReader::new(log, RECORD_LIMIT)
            .read(UtcInterval::new(now - jiff::SignedDuration::from_hours(24), now).unwrap());

        assert!(
            matches!(
                read,
                SessionHistoryRead::Complete(_) | SessionHistoryRead::Truncated
            ),
            "{read:?}"
        );
    }
}
