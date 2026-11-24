//! The session boundaries of the Winlogon log: when the user's session was opened, locked,
//! unlocked and closed before the app started watching it.
//!
//! Winlogon notifies its subscribers of every session change and logs event 811 each time one
//! starts handling a notification. The log is on by default and readable without elevation.
//! Only the query of the log is Windows only; the event XML is parsed here, and
//! `bientot_platform::session_history` orders and records what it describes.

#![cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "only the Windows Winlogon log decodes records outside the tests"
    )
)]

mod event_xml;
#[cfg(windows)]
mod native;

use bientot_domain::accounting::SESSION_HISTORY_SOURCE_ID;
use bientot_platform::power_history::HistoryUnavailable;
use bientot_platform::session_history::{LockHistory, SessionBoundaries, SessionBoundaryLog};
use jiff::Timestamp;

use crate::event_log::{CollectedRecords, LogUnavailable};

#[cfg(windows)]
pub use native::current_user_log;

/// The Winlogon notifications of one user in the `Microsoft-Windows-Winlogon/Operational` log.
pub(crate) trait WinlogonEventLog {
    /// The user's security identifier, as `S-1-5-21-…`.
    fn user_id(&self) -> &str;

    /// The records of the notifications that began at `since` or later.
    fn read(
        &self,
        since: Timestamp,
        record_limit: usize,
    ) -> Result<CollectedRecords, LogUnavailable>;
}

/// The session boundaries the records of a [`WinlogonEventLog`] describe for its user.
/// Notifications of other users are left out.
pub(crate) struct WinlogonBoundaries<L>(pub(crate) L);

impl<L: WinlogonEventLog> SessionBoundaryLog for WinlogonBoundaries<L> {
    fn origin(&self) -> &'static str {
        SESSION_HISTORY_SOURCE_ID
    }

    fn lock_history(&self) -> LockHistory {
        LockHistory::Recorded
    }

    fn read(
        &self,
        since: Timestamp,
        record_limit: usize,
    ) -> Result<SessionBoundaries, HistoryUnavailable> {
        let records = match self.0.read(since, record_limit) {
            Ok(CollectedRecords::Complete(records)) => records,
            Ok(CollectedRecords::Truncated) => {
                return Ok(SessionBoundaries {
                    boundaries: Vec::new(),
                    complete: false,
                });
            }
            Err(LogUnavailable) => {
                return Err(HistoryUnavailable::Failed(
                    "the Winlogon log could not be queried or read".to_owned(),
                ));
            }
        };
        let notifications = records
            .iter()
            .map(|xml| event_xml::parse_notification(xml))
            .collect::<Option<Vec<_>>>()
            .ok_or(HistoryUnavailable::Malformed)?;
        Ok(SessionBoundaries {
            boundaries: notifications
                .into_iter()
                .flatten()
                .filter(|notification| notification.user_id == self.0.user_id())
                .map(|notification| notification.boundary)
                .collect(),
            complete: true,
        })
    }
}

#[cfg(test)]
mod tests;
