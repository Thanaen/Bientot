//! Session history: when the user's session was opened, locked, unlocked and closed before the
//! app started watching it. A [`SessionBoundaryLog`] reads the boundaries; they are ordered and
//! recorded here.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use bientot_domain::accounting::{
    OperationKey, SessionState, WorkdayTransition, WorkdayTransitionKind,
};
use bientot_domain::intervals::UtcInterval;
use bientot_domain::time::WorkdayWindow;
use bientot_store::sessions::{
    AtomicWorkdayTransition, SESSION_HISTORY_KEY_PREFIX, SessionEventError, SessionEventRepository,
};
use jiff::{SignedDuration, Timestamp};

use crate::dotnet_ticks;
use crate::lifecycle::probe_within;
use crate::power_history::HistoryUnavailable;

/// The most matching records one read keeps; a log with more reads as
/// [`SessionHistoryRead::Truncated`].
pub const RECORD_LIMIT: usize = 4096;

/// How far before the window a read looks for the state the session was in when it began.
pub const LOOKBACK: SignedDuration = SignedDuration::from_hours(24);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SessionBoundaryKind {
    Logon,
    Logoff,
    Lock,
    Unlock,
}

impl SessionBoundaryKind {
    /// The state the session is in from the boundary on. A closed session tells nothing about
    /// the user, so it reads as unknown.
    fn state(self) -> SessionState {
        match self {
            Self::Logon | Self::Unlock => SessionState::Unlocked,
            Self::Lock => SessionState::Locked,
            Self::Logoff => SessionState::Unknown,
        }
    }
}

/// One change of the session, already decoded.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionBoundary {
    pub kind: SessionBoundaryKind,
    pub at: Timestamp,
    /// Stable identity of the source record, for idempotent recording.
    pub source_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionBoundaries {
    pub boundaries: Vec<SessionBoundary>,
    /// False when the source stopped at `record_limit` or lost records.
    pub complete: bool,
}

/// Whether a silence in the log means « nothing happened ».
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockHistory {
    /// Every lock in the window is in the log (Windows).
    Recorded,
    /// The platform keeps no record of locks (Linux). The recovery then follows D-026.
    NotRecorded,
}

/// The boundaries of the user's session a log of the system recorded.
pub trait SessionBoundaryLog {
    /// Ledger origin of what this log records, e.g. "windows-session-history".
    fn origin(&self) -> &'static str;
    fn lock_history(&self) -> LockHistory;
    /// The boundaries at `since` or later.
    fn read(
        &self,
        since: Timestamp,
        record_limit: usize,
    ) -> Result<SessionBoundaries, HistoryUnavailable>;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionHistory {
    /// The last boundary within [`LOOKBACK`] before the window, which tells the state of the
    /// session when the window began.
    pub before_window: Option<SessionBoundary>,
    /// The boundaries inside the window, oldest first.
    pub in_window: Vec<SessionBoundary>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionHistoryUnavailable {
    MalformedEvent,
    LogUnavailable,
    Timeout,
    PersistenceFailed,
    RecoveryFailed,
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
            Self::Unavailable(Timeout) => "windows-winlogon-log-timeout",
            Self::Unavailable(PersistenceFailed) => "windows-session-history-persistence-failed",
            Self::Unavailable(RecoveryFailed) => "windows-session-history-recovery-failed",
        })
    }
}

/// Every subscriber handles each notification, so one lock is logged several times within a
/// few milliseconds. Of a run of boundaries of one kind, only the first changed the session.
/// Boundaries of one instant are ordered by source key, shorter first, so that the numbered
/// records of the Windows log keep their numeric order.
pub fn collapse(boundaries: impl IntoIterator<Item = SessionBoundary>) -> Vec<SessionBoundary> {
    let mut ordered: Vec<_> = boundaries.into_iter().collect();
    ordered.sort_by(|left, right| {
        (left.at, left.source_key.len(), &left.source_key, left.kind).cmp(&(
            right.at,
            right.source_key.len(),
            &right.source_key,
            right.kind,
        ))
    });
    ordered.dedup_by(|later, earlier| later.kind == earlier.kind);
    ordered
}

/// The session history of a window, read from a [`SessionBoundaryLog`].
pub struct SessionHistoryReader<L> {
    log: L,
    record_limit: usize,
}

impl<L: SessionBoundaryLog> SessionHistoryReader<L> {
    pub fn new(log: L, record_limit: usize) -> Self {
        Self { log, record_limit }
    }

    /// The boundaries inside `window`, and the last one in the [`LOOKBACK`] before it.
    pub fn read(&self, window: UtcInterval) -> SessionHistoryRead {
        let since = window.start() - LOOKBACK;
        let boundaries = match self.log.read(since, self.record_limit) {
            Ok(SessionBoundaries {
                boundaries,
                complete: true,
            }) => boundaries,
            Ok(SessionBoundaries {
                complete: false, ..
            }) => return SessionHistoryRead::Truncated,
            Err(HistoryUnavailable::Malformed) => {
                return SessionHistoryRead::Unavailable(SessionHistoryUnavailable::MalformedEvent);
            }
            Err(
                HistoryUnavailable::AccessDenied
                | HistoryUnavailable::NoSource
                | HistoryUnavailable::Failed(_),
            ) => {
                return SessionHistoryRead::Unavailable(SessionHistoryUnavailable::LogUnavailable);
            }
        };
        let boundaries = collapse(
            boundaries
                .into_iter()
                .filter(|boundary| since <= boundary.at && boundary.at < window.end()),
        );
        let (mut before, inside): (Vec<_>, Vec<_>) = boundaries
            .into_iter()
            .partition(|boundary| boundary.at < window.start());
        SessionHistoryRead::Complete(SessionHistory {
            before_window: before.pop(),
            in_window: inside,
        })
    }
}

/// Records, on the first launch, how the session went from the start of the workday until the
/// app started watching it, so that the time it was unlocked counts provisionally (issue #75).
pub struct SessionHistoryRecovery<'a> {
    events: SessionEventRepository<'a>,
    read_timeout: Duration,
}

impl<'a> SessionHistoryRecovery<'a> {
    /// How long startup waits for the log; see [`probe_within`].
    pub const READ_TIMEOUT: Duration = Duration::from_secs(3);

    pub fn new(events: SessionEventRepository<'a>, read_timeout: Duration) -> Self {
        Self {
            events,
            read_timeout,
        }
    }

    /// Reads the history from the start of `window` to `now` with `read` and records each of its
    /// boundaries, all in one transaction, under a key that names its source key and instant, so
    /// that reading it again records nothing new. A history that fails to record is returned as
    /// unavailable.
    pub fn recover(
        &self,
        window: &WorkdayWindow,
        now: Timestamp,
        read: impl FnOnce(UtcInterval) -> SessionHistoryRead + Send + 'static,
    ) -> SessionHistoryRead {
        let failed = SessionHistoryRead::Unavailable(SessionHistoryUnavailable::RecoveryFailed);
        let Ok(before_now) = UtcInterval::new(window.start, now.min(window.end)) else {
            return failed;
        };
        let history = probe_within(self.read_timeout, move || {
            catch_unwind(AssertUnwindSafe(|| read(before_now))).unwrap_or(failed)
        })
        .unwrap_or(SessionHistoryRead::Unavailable(
            SessionHistoryUnavailable::Timeout,
        ));
        match history {
            SessionHistoryRead::Complete(history) => match self.record(&history) {
                Ok(()) => SessionHistoryRead::Complete(history),
                Err(_) => {
                    SessionHistoryRead::Unavailable(SessionHistoryUnavailable::PersistenceFailed)
                }
            },
            incomplete => incomplete,
        }
    }

    fn record(&self, history: &SessionHistory) -> Result<(), SessionEventError> {
        let transitions: Vec<_> = history
            .in_window
            .iter()
            .map(|boundary| AtomicWorkdayTransition {
                operation_key: history_key(boundary),
                transition: WorkdayTransition::new(
                    boundary.at,
                    "UTC",
                    WorkdayTransitionKind::SessionObserved(boundary.kind.state()),
                )
                .expect("UTC is not blank"),
            })
            .collect();
        self.events.record_atomically(&transitions)?;
        Ok(())
    }
}

fn history_key(boundary: &SessionBoundary) -> OperationKey {
    let state = match boundary.kind.state() {
        SessionState::Unlocked => "unlocked",
        SessionState::Locked => "locked",
        SessionState::Unknown => "unknown",
    };
    OperationKey::new(format!(
        "{SESSION_HISTORY_KEY_PREFIX}{state}:{}:{}",
        boundary.source_key,
        dotnet_ticks(boundary.at)
    ))
    .expect("the key starts with its prefix")
}

#[cfg(test)]
mod tests;
