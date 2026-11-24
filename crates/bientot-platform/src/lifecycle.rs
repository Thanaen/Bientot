//! Session lock and unlock, suspend and resume, ported from `Services/LifecycleEventRecorder.cs`.
//! The recording below is platform independent.

use std::sync::mpsc;
use std::time::Duration;

use bientot_domain::accounting::{
    self, BlankIdentifier, OperationKey, SessionState, WorkdayTransition, WorkdayTransitionKind,
};
use bientot_store::sessions::{RecordTransitionOutcome, SessionEventError, SessionEventRepository};
use jiff::Timestamp;

use crate::{PowerState, dotnet_ticks};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleChange {
    Session(SessionState),
    Power(PowerState),
}

/// A change with the instant and the local time zone it was observed in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleTransition {
    pub change: LifecycleChange,
    pub occurred_at: Timestamp,
    pub time_zone_id: String,
}

/// What the host does around each recorded change.
pub trait LifecycleHooks {
    /// Called before a watched power change is recorded: the host hands it to the built-in
    /// activity source, so that suspend stores the settled boundary before the computer sleeps.
    fn power_changing(&mut self, state: PowerState);

    /// Called after each change that is new to the ledger.
    fn refresh(&mut self);
}

#[derive(Debug)]
pub enum LifecycleError {
    Storage(SessionEventError),
    BlankTimeZone(BlankIdentifier),
}

impl std::fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(f),
            Self::BlankTimeZone(_) => f.write_str("the local time zone has no identifier"),
        }
    }
}

impl std::error::Error for LifecycleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::BlankTimeZone(error) => Some(error),
        }
    }
}

impl From<SessionEventError> for LifecycleError {
    fn from(error: SessionEventError) -> Self {
        Self::Storage(error)
    }
}

/// Runs `probe` on a thread of its own and gives up after `timeout`, so that a query that hangs
/// cannot hold startup. An abandoned probe ends on its own when the query returns.
pub fn probe_within<T: Send + 'static>(
    timeout: Duration,
    probe: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("worktime-lifecycle-probe".into())
        .spawn(move || {
            let _ = sender.send(probe());
        })
        .ok()?;
    receiver.recv_timeout(timeout).ok()
}

/// Records session and power changes in the ledger: source `windows-session` or
/// `windows-power`, one operation key per change, its instant and its time zone.
pub struct LifecycleEventRecorder<'a> {
    events: SessionEventRepository<'a>,
}

impl<'a> LifecycleEventRecorder<'a> {
    /// How long startup waits for the current session state; see [`probe_within`].
    pub const PREPARE_TIMEOUT: Duration = Duration::from_secs(3);

    pub fn new(events: SessionEventRepository<'a>) -> Self {
        Self { events }
    }

    /// Records the state at startup, at one instant: `session` as probed, unless unknown, and
    /// the power as active, since this process is running.
    pub fn prepare(
        &self,
        session: SessionState,
        occurred_at: Timestamp,
        time_zone_id: &str,
        hooks: &mut dyn LifecycleHooks,
    ) -> Result<(), LifecycleError> {
        for change in [
            LifecycleChange::Session(session),
            LifecycleChange::Power(PowerState::Active),
        ] {
            self.record_change(change, occurred_at, time_zone_id, hooks)?;
        }
        Ok(())
    }

    /// Records a watched change, handing a power change to the hooks first.
    pub fn record(
        &self,
        transition: &LifecycleTransition,
        hooks: &mut dyn LifecycleHooks,
    ) -> Result<(), LifecycleError> {
        if let LifecycleChange::Power(state) = transition.change {
            hooks.power_changing(state);
        }
        self.record_change(
            transition.change,
            transition.occurred_at,
            &transition.time_zone_id,
            hooks,
        )
    }

    fn record_change(
        &self,
        change: LifecycleChange,
        occurred_at: Timestamp,
        time_zone_id: &str,
        hooks: &mut dyn LifecycleHooks,
    ) -> Result<(), LifecycleError> {
        let Some((lane, state, kind)) = ledger_form(change) else {
            return Ok(());
        };
        let transition = WorkdayTransition::new(occurred_at, time_zone_id, kind)
            .map_err(LifecycleError::BlankTimeZone)?;
        let key = OperationKey::new(format!(
            "lifecycle:{lane}:{}:{state}:{}",
            dotnet_ticks(occurred_at),
            transition.time_zone_id()
        ))
        .expect("the key starts with its lane");
        if self.events.record(key, transition)?.outcome == RecordTransitionOutcome::Inserted {
            hooks.refresh();
        }
        Ok(())
    }
}

/// The lane and state of the .NET operation key, and the transition the ledger stores. An
/// unknown session is not recorded.
fn ledger_form(
    change: LifecycleChange,
) -> Option<(&'static str, &'static str, WorkdayTransitionKind)> {
    use WorkdayTransitionKind::{PowerObserved, SessionObserved};
    match change {
        LifecycleChange::Session(SessionState::Unknown) => None,
        LifecycleChange::Session(SessionState::Locked) => {
            Some(("session", "locked", SessionObserved(SessionState::Locked)))
        }
        LifecycleChange::Session(SessionState::Unlocked) => Some((
            "session",
            "unlocked",
            SessionObserved(SessionState::Unlocked),
        )),
        LifecycleChange::Power(PowerState::Active) => Some((
            "power",
            "active",
            PowerObserved(accounting::PowerState::Active),
        )),
        LifecycleChange::Power(PowerState::Suspended) => Some((
            "power",
            "suspended",
            PowerObserved(accounting::PowerState::Suspended),
        )),
    }
}

#[cfg(test)]
mod tests;
