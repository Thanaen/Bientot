//! Session history from the Winlogon log: when the user's session was opened, locked, unlocked
//! and closed before the app started watching it.
//!
//! Winlogon notifies its subscribers of every session change and logs event 811 each time one
//! starts handling a notification. The log is on by default and readable without elevation.
//! Only the query of the log is Windows only; the event XML is parsed and ordered here.

mod event_xml;
#[cfg(windows)]
mod native;

use std::num::NonZeroU64;

use bientot_domain::intervals::UtcInterval;
use jiff::{SignedDuration, Timestamp};

pub use crate::event_log::{CollectedRecords, LogUnavailable};

#[cfg(windows)]
pub use native::{WinlogonLog, current_user_id};

/// The most matching records one read keeps; a log with more reads as
/// [`SessionHistoryRead::Truncated`].
pub const RECORD_LIMIT: usize = 4096;

/// How far before the window a read looks for the state the session was in when it began.
pub const LOOKBACK: SignedDuration = SignedDuration::from_hours(24);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SessionHistoryBoundaryKind {
    Logon,
    Logoff,
    Lock,
    Unlock,
}

impl SessionHistoryBoundaryKind {
    /// The kind of a Winlogon notification, by the number its `Event` data gives it. A session
    /// disconnected by Switch User reads as locked, and reconnected as unlocked. Screen saver
    /// and system notifications are not boundaries.
    fn from_notification(notification: u64) -> Option<Self> {
        match notification {
            2 | 12 => Some(Self::Logon),
            3 | 13 => Some(Self::Logoff),
            4 | 8 => Some(Self::Lock),
            5 | 9 => Some(Self::Unlock),
            _ => None,
        }
    }
}

/// One change of the session as the log records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SessionHistoryBoundary {
    pub kind: SessionHistoryBoundaryKind,
    /// `EventRecordID`, which the log reuses after it is cleared, so it identifies a boundary
    /// only together with the instant.
    pub record_id: NonZeroU64,
    pub occurred_at: Timestamp,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionHistory {
    /// The last boundary within [`LOOKBACK`] before the window, which tells the state of the
    /// session when the window began.
    pub before_window: Option<SessionHistoryBoundary>,
    /// The boundaries inside the window, oldest first.
    pub in_window: Vec<SessionHistoryBoundary>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionHistoryUnavailable {
    MalformedEvent,
    LogUnavailable,
}

/// The outcome of one read. An incomplete read carries no evidence at all, since a missing
/// record could hide a lock between two of the others.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionHistoryRead {
    Complete(SessionHistory),
    /// More than [`RECORD_LIMIT`] records matched.
    Truncated,
    Unavailable(SessionHistoryUnavailable),
}

impl SessionHistoryRead {
    /// The reason to log when the read carries no history.
    pub fn reason(&self) -> Option<&'static str> {
        use SessionHistoryUnavailable::*;
        Some(match self {
            Self::Complete(_) => return None,
            Self::Truncated => "windows-winlogon-log-record-limit",
            Self::Unavailable(MalformedEvent) => "windows-winlogon-log-malformed-event",
            Self::Unavailable(LogUnavailable) => "windows-winlogon-log-unavailable",
        })
    }
}

/// The Winlogon notifications of one user in the `Microsoft-Windows-Winlogon/Operational` log.
pub trait WinlogonEventLog {
    /// The records of the notifications that began at `since` or later.
    fn read(
        &self,
        since: Timestamp,
        record_limit: usize,
    ) -> Result<CollectedRecords, LogUnavailable>;
}

/// Every subscriber handles each notification, so one lock is logged several times within a
/// few milliseconds. Of a run of boundaries of one kind, only the first changed the session.
pub fn collapse(
    boundaries: impl IntoIterator<Item = SessionHistoryBoundary>,
) -> Vec<SessionHistoryBoundary> {
    let mut ordered: Vec<_> = boundaries.into_iter().collect();
    ordered.sort_by_key(|boundary| (boundary.occurred_at, boundary.record_id, boundary.kind));
    ordered.dedup_by(|later, earlier| later.kind == earlier.kind);
    ordered
}

/// The session history of a window, read from a [`WinlogonEventLog`] for one user.
pub struct SessionHistoryReader<L> {
    log: L,
    user_id: String,
    record_limit: usize,
}

impl<L: WinlogonEventLog> SessionHistoryReader<L> {
    /// `user_id` is the user's security identifier, as `S-1-5-21-…`; notifications of other
    /// users are left out.
    pub fn new(log: L, user_id: impl Into<String>, record_limit: usize) -> Self {
        Self {
            log,
            user_id: user_id.into(),
            record_limit,
        }
    }

    /// The boundaries inside `window`, and the last one in the [`LOOKBACK`] before it.
    pub fn read(&self, window: UtcInterval) -> SessionHistoryRead {
        let since = window.start() - LOOKBACK;
        let records = match self.log.read(since, self.record_limit) {
            Ok(CollectedRecords::Complete(records)) => records,
            Ok(CollectedRecords::Truncated) => return SessionHistoryRead::Truncated,
            Err(LogUnavailable) => {
                return SessionHistoryRead::Unavailable(SessionHistoryUnavailable::LogUnavailable);
            }
        };
        let Some(notifications) = records
            .iter()
            .map(|xml| event_xml::parse_notification(xml))
            .collect::<Option<Vec<_>>>()
        else {
            return SessionHistoryRead::Unavailable(SessionHistoryUnavailable::MalformedEvent);
        };
        let boundaries = collapse(
            notifications
                .into_iter()
                .flatten()
                .filter(|notification| notification.user_id == self.user_id)
                .map(|notification| notification.boundary)
                .filter(|boundary| {
                    since <= boundary.occurred_at && boundary.occurred_at < window.end()
                }),
        );
        let (before, inside): (Vec<_>, Vec<_>) = boundaries
            .into_iter()
            .partition(|boundary| boundary.occurred_at < window.start());
        SessionHistoryRead::Complete(SessionHistory {
            before_window: before.last().copied(),
            in_window: inside,
        })
    }
}

#[cfg(test)]
mod tests;
