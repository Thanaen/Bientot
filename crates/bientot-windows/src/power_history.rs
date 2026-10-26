//! Power history recovery from the System event log, ported from
//! `Power/WindowsPowerHistoryPort.cs`, `Power/PowerHistoryPairer.cs`,
//! `Abstractions/Power/IPowerHistoryPort.cs` and `Services/PowerHistoryRecoveryService.cs`.
//! See `docs/power-history-recovery.md`.
//!
//! The app is not running while the computer is off or asleep, so at startup it reads what the
//! kernel logged meanwhile and records the verified intervals as if it had watched them. Only
//! the query of the log is Windows only; the event XML is parsed, paired and recorded here.

mod event_xml;
#[cfg(windows)]
mod native;

use std::collections::HashSet;
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use bientot_domain::accounting::{
    OperationKey, PowerState, SessionState, WorkdayTransition, WorkdayTransitionKind,
};
use bientot_domain::intervals::UtcInterval;
use bientot_domain::time::WorkdayWindow;
use bientot_store::sessions::{AtomicWorkdayTransition, SessionEventError, SessionEventRepository};
use jiff::Timestamp;

use crate::dotnet_ticks;
pub use crate::event_log::{CollectedRecords, collect};
use crate::lifecycle::probe_within;

#[cfg(windows)]
pub use native::SystemLog;

const KERNEL_GENERAL: &str = "Microsoft-Windows-Kernel-General";
const KERNEL_POWER: &str = "Microsoft-Windows-Kernel-Power";

/// The most matching records one read keeps; a log with more reads as
/// [`PowerHistoryRead::Truncated`].
pub const RECORD_LIMIT: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PowerHistoryBoundaryKind {
    Shutdown,
    Boot,
    Suspend,
    Resume,
}

impl PowerHistoryBoundaryKind {
    const ALL: [Self; 4] = [Self::Shutdown, Self::Boot, Self::Suspend, Self::Resume];

    /// The provider and event ID that log this boundary.
    pub fn event(self) -> (&'static str, u32) {
        match self {
            Self::Shutdown => (KERNEL_GENERAL, 13),
            Self::Boot => (KERNEL_GENERAL, 12),
            Self::Suspend => (KERNEL_POWER, 42),
            Self::Resume => (KERNEL_POWER, 107),
        }
    }

    fn from_event(provider: &str, event_id: u32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.event() == (provider, event_id))
    }
}

/// One boundary as the log records it. The kind names its provider and event ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PowerHistoryBoundary {
    pub kind: PowerHistoryBoundaryKind,
    /// `EventRecordID`, which the log reuses after it is cleared, so it identifies a boundary
    /// only together with the instant.
    pub record_id: NonZeroU64,
    pub occurred_at: Timestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerHistoryIntervalKind {
    PoweredOff,
    Suspended,
}

impl PowerHistoryIntervalKind {
    fn boundaries(self) -> (PowerHistoryBoundaryKind, PowerHistoryBoundaryKind) {
        match self {
            Self::PoweredOff => (
                PowerHistoryBoundaryKind::Shutdown,
                PowerHistoryBoundaryKind::Boot,
            ),
            Self::Suspended => (
                PowerHistoryBoundaryKind::Suspend,
                PowerHistoryBoundaryKind::Resume,
            ),
        }
    }
}

/// A shutdown and the boot that ended it, or a suspend and its resume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerHistoryInterval {
    kind: PowerHistoryIntervalKind,
    start: PowerHistoryBoundary,
    end: PowerHistoryBoundary,
}

impl PowerHistoryInterval {
    /// `None` unless the boundaries are of the kinds that open and close `kind` and `end` comes
    /// after `start`.
    pub fn new(
        kind: PowerHistoryIntervalKind,
        start: PowerHistoryBoundary,
        end: PowerHistoryBoundary,
    ) -> Option<Self> {
        ((start.kind, end.kind) == kind.boundaries() && start.occurred_at < end.occurred_at)
            .then_some(Self { kind, start, end })
    }

    pub fn kind(&self) -> PowerHistoryIntervalKind {
        self.kind
    }

    pub fn start(&self) -> PowerHistoryBoundary {
        self.start
    }

    pub fn end(&self) -> PowerHistoryBoundary {
        self.end
    }

    pub fn interval(&self) -> UtcInterval {
        UtcInterval::new(self.start.occurred_at, self.end.occurred_at)
            .expect("an interval ends after it starts")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PowerHistory {
    pub intervals: Vec<PowerHistoryInterval>,
    /// Every boot, those that end a powered-off interval included: a boot ends whatever
    /// session was open before it.
    pub boots: Vec<PowerHistoryBoundary>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerHistoryUnavailable {
    MalformedEvent,
    LogUnavailable,
    Timeout,
    PersistenceFailed,
    RecoveryFailed,
}

/// The outcome of one read. An incomplete read carries no evidence at all, since a missing
/// record could pair any of the others differently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PowerHistoryRead {
    Complete(PowerHistory),
    /// More than [`RECORD_LIMIT`] records matched.
    Truncated,
    Unavailable(PowerHistoryUnavailable),
}

impl PowerHistoryRead {
    /// The reason the .NET app logs with `power_history.recovery_unavailable`.
    pub fn reason(&self) -> Option<&'static str> {
        use PowerHistoryUnavailable::*;
        Some(match self {
            Self::Complete(_) => return None,
            Self::Truncated => "windows-system-log-record-limit",
            Self::Unavailable(MalformedEvent) => "windows-system-log-malformed-event",
            Self::Unavailable(LogUnavailable) => "windows-system-log-unavailable",
            Self::Unavailable(Timeout) => "windows-system-log-timeout",
            Self::Unavailable(PersistenceFailed) => "windows-power-history-persistence-failed",
            Self::Unavailable(RecoveryFailed) => "windows-power-history-recovery-failed",
        })
    }
}

/// Pairs each shutdown with the next boot and each suspend with the next resume, in the order
/// of their instants. Anything ambiguous pairs nothing: two boundaries at one instant, a second
/// shutdown or suspend before its closer, a suspend or resume while a shutdown is open, or a
/// boot while a suspend is open. Exact duplicates count once.
pub fn pair(boundaries: impl IntoIterator<Item = PowerHistoryBoundary>) -> PowerHistory {
    #[derive(Clone, Copy)]
    enum Pending {
        Clear,
        Open(PowerHistoryBoundary),
        Conflict,
    }
    use Pending::{Clear, Conflict, Open};

    let mut ordered: Vec<_> = boundaries.into_iter().collect();
    ordered.sort_by_key(|boundary| (boundary.occurred_at, boundary.record_id, boundary.kind));
    ordered.dedup();

    let (mut intervals, mut boots) = (Vec::new(), Vec::new());
    let mut close = |kind, start, end| {
        let interval = PowerHistoryInterval::new(kind, start, end)
            .expect("an open boundary precedes the instant that closes it");
        intervals.push(interval);
    };
    let (mut shutdown, mut suspend) = (Clear, Clear);
    for at_one_instant in ordered.chunk_by(|left, right| left.occurred_at == right.occurred_at) {
        let &[boundary] = at_one_instant else {
            (shutdown, suspend) = (Conflict, Conflict);
            continue;
        };
        match boundary.kind {
            PowerHistoryBoundaryKind::Shutdown => {
                suspend = Clear;
                shutdown = match shutdown {
                    Clear => Open(boundary),
                    _ => Conflict,
                };
            }
            PowerHistoryBoundaryKind::Boot => {
                boots.push(boundary);
                suspend = Clear;
                if let Open(start) = shutdown {
                    close(PowerHistoryIntervalKind::PoweredOff, start, boundary);
                }
                shutdown = Clear;
            }
            PowerHistoryBoundaryKind::Suspend => {
                if !matches!(shutdown, Clear) {
                    shutdown = Conflict;
                    suspend = Clear;
                } else {
                    suspend = match suspend {
                        Clear => Open(boundary),
                        _ => Conflict,
                    };
                }
            }
            PowerHistoryBoundaryKind::Resume => {
                if !matches!(shutdown, Clear) {
                    shutdown = Conflict;
                } else if let Open(start) = suspend {
                    close(PowerHistoryIntervalKind::Suspended, start, boundary);
                }
                suspend = Clear;
            }
        }
    }
    PowerHistory { intervals, boots }
}

/// The System log could not be queried or read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemLogUnavailable;

/// The kernel's boot, shutdown, suspend and resume records in the System log.
pub trait SystemEventLog {
    fn read(&self, record_limit: usize) -> Result<CollectedRecords, SystemLogUnavailable>;
}

/// The power history of a window, read from a [`SystemEventLog`].
pub struct PowerHistoryReader<L> {
    log: L,
    record_limit: usize,
}

impl<L: SystemEventLog> PowerHistoryReader<L> {
    pub fn new(log: L, record_limit: usize) -> Self {
        Self { log, record_limit }
    }

    /// The intervals that overlap `window` and the boots inside it, paired across the whole
    /// log so that an interval that began long before the window is still found.
    pub fn read(&self, window: UtcInterval) -> PowerHistoryRead {
        let records = match self.log.read(self.record_limit) {
            Ok(CollectedRecords::Complete(records)) => records,
            Ok(CollectedRecords::Truncated) => return PowerHistoryRead::Truncated,
            Err(SystemLogUnavailable) => {
                return PowerHistoryRead::Unavailable(PowerHistoryUnavailable::LogUnavailable);
            }
        };
        let Some(boundaries) = records
            .iter()
            .map(|xml| event_xml::parse_boundary(xml))
            .collect::<Option<Vec<_>>>()
        else {
            return PowerHistoryRead::Unavailable(PowerHistoryUnavailable::MalformedEvent);
        };
        let history = pair(boundaries);
        PowerHistoryRead::Complete(PowerHistory {
            intervals: history
                .intervals
                .into_iter()
                .filter(|interval| interval.interval().overlaps(window))
                .collect(),
            boots: history
                .boots
                .into_iter()
                .filter(|boot| window.contains(boot.occurred_at))
                .collect(),
        })
    }
}

/// Records the power history of the current workday at startup, before the first summary.
pub struct PowerHistoryRecovery<'a> {
    events: SessionEventRepository<'a>,
    read_timeout: Duration,
}

impl<'a> PowerHistoryRecovery<'a> {
    /// How long startup waits for the log; see [`probe_within`].
    pub const READ_TIMEOUT: Duration = Duration::from_secs(3);

    pub fn new(events: SessionEventRepository<'a>, read_timeout: Duration) -> Self {
        Self {
            events,
            read_timeout,
        }
    }

    /// Reads the history of `window` with `read` and records it in UTC: each interval with
    /// both of its ends in one transaction, a session of unknown state after each boot, and a
    /// session and power of unknown state at the window start, so that a lock or suspend
    /// recorded before the window does not stand in for history that could not be read. The
    /// unknown power is left out when a recorded interval already covers the window start.
    ///
    /// Every key names its boundary's provider, event ID, record ID and instant, so reading the
    /// same history again records nothing new. A history that fails to record is returned as
    /// unavailable; only the unknown states at the window start fail the call.
    pub fn recover(
        &self,
        window: &WorkdayWindow,
        read: impl FnOnce(UtcInterval) -> PowerHistoryRead + Send + 'static,
    ) -> Result<PowerHistoryRead, SessionEventError> {
        let history = self.read(window, read);
        self.record_knowledge_boundary(
            "session",
            window.start,
            WorkdayTransitionKind::SessionObserved(SessionState::Unknown),
        )?;
        let history = match history {
            PowerHistoryRead::Complete(history) => match self.record_history(&history, window) {
                Ok(()) => PowerHistoryRead::Complete(history),
                Err(_) => PowerHistoryRead::Unavailable(PowerHistoryUnavailable::PersistenceFailed),
            },
            incomplete => incomplete,
        };
        let covers_window_start = matches!(&history, PowerHistoryRead::Complete(history)
        if history.intervals.iter().any(|interval| {
            interval.start.occurred_at <= window.start && interval.end.occurred_at > window.start
        }));
        if !covers_window_start {
            self.record_knowledge_boundary(
                "power",
                window.start,
                WorkdayTransitionKind::PowerObserved(PowerState::Unknown),
            )?;
        }
        Ok(history)
    }

    fn read(
        &self,
        window: &WorkdayWindow,
        read: impl FnOnce(UtcInterval) -> PowerHistoryRead + Send + 'static,
    ) -> PowerHistoryRead {
        let failed = PowerHistoryRead::Unavailable(PowerHistoryUnavailable::RecoveryFailed);
        let Ok(window) = UtcInterval::new(window.start, window.end) else {
            return failed;
        };
        probe_within(self.read_timeout, move || {
            catch_unwind(AssertUnwindSafe(|| read(window))).unwrap_or(failed)
        })
        .unwrap_or(PowerHistoryRead::Unavailable(
            PowerHistoryUnavailable::Timeout,
        ))
    }

    fn record_history(
        &self,
        history: &PowerHistory,
        window: &WorkdayWindow,
    ) -> Result<(), SessionEventError> {
        for interval in &history.intervals {
            self.events
                .record_atomically(&interval_transitions(interval, window))?;
        }
        let paired_boots: HashSet<_> = history
            .intervals
            .iter()
            .filter(|interval| interval.kind == PowerHistoryIntervalKind::PoweredOff)
            .map(|interval| interval.end)
            .collect();
        for boot in history
            .boots
            .iter()
            .filter(|boot| !paired_boots.contains(boot))
        {
            self.events.record(
                history_key("session", "unknown-at-boot", boot, None),
                utc(
                    boot.occurred_at,
                    WorkdayTransitionKind::SessionObserved(SessionState::Unknown),
                ),
            )?;
        }
        Ok(())
    }

    fn record_knowledge_boundary(
        &self,
        lane: &str,
        occurred_at: Timestamp,
        kind: WorkdayTransitionKind,
    ) -> Result<(), SessionEventError> {
        let key = OperationKey::new(format!(
            "startup-knowledge:{lane}:{}",
            dotnet_ticks(occurred_at)
        ))
        .expect("the key starts with its prefix");
        self.events.record(key, utc(occurred_at, kind))?;
        Ok(())
    }
}

/// Suspended at the start and active at the end, suspended again at the window start when the
/// interval spans it, and a session of unknown state after a boot.
fn interval_transitions(
    interval: &PowerHistoryInterval,
    window: &WorkdayWindow,
) -> Vec<AtomicWorkdayTransition> {
    let power = |state| WorkdayTransitionKind::PowerObserved(state);
    let mut transitions = vec![
        AtomicWorkdayTransition {
            operation_key: history_key("power", "suspended", &interval.start, None),
            transition: utc(interval.start.occurred_at, power(PowerState::Suspended)),
        },
        AtomicWorkdayTransition {
            operation_key: history_key("power", "active", &interval.end, None),
            transition: utc(interval.end.occurred_at, power(PowerState::Active)),
        },
    ];
    if interval.start.occurred_at < window.start && interval.end.occurred_at > window.start {
        transitions.push(AtomicWorkdayTransition {
            operation_key: history_key(
                "power",
                "suspended-at-window-start",
                &interval.start,
                Some(window.start),
            ),
            transition: utc(window.start, power(PowerState::Suspended)),
        });
    }
    if interval.kind == PowerHistoryIntervalKind::PoweredOff {
        transitions.push(AtomicWorkdayTransition {
            operation_key: history_key("session", "unknown", &interval.end, None),
            transition: utc(
                interval.end.occurred_at,
                WorkdayTransitionKind::SessionObserved(SessionState::Unknown),
            ),
        });
    }
    transitions
}

fn history_key(
    lane: &str,
    state: &str,
    boundary: &PowerHistoryBoundary,
    window_start: Option<Timestamp>,
) -> OperationKey {
    let (provider, event_id) = boundary.kind.event();
    let mut key = format!(
        "power-history:{lane}:{state}:{}:{event_id}:{}:{}",
        provider.to_lowercase(),
        boundary.record_id,
        dotnet_ticks(boundary.occurred_at)
    );
    if let Some(window_start) = window_start {
        key += &format!(":{}", dotnet_ticks(window_start));
    }
    OperationKey::new(key).expect("the key starts with its prefix")
}

fn utc(occurred_at: Timestamp, kind: WorkdayTransitionKind) -> WorkdayTransition {
    WorkdayTransition::new(occurred_at, "UTC", kind).expect("UTC is not blank")
}

#[cfg(test)]
mod tests;
