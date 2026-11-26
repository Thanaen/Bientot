//! Session lock and unlock, suspend and resume, from logind (research.md, R7;
//! contracts/linux-backends.md, « Lock, sleep, session: logind »).
//!
//! logind reports the session's `LockedHint` and `Active` and the manager's `PrepareForSleep`.
//! A desktop that never sets `LockedHint` may still announce its lock screen through the
//! screensaver's `ActiveChanged`. [`SessionTracker`] turns these signals into the four changes
//! Windows records and no other; `LogindSource`, on Linux, receives them on a thread of its own
//! and finds the session of this process.

#[cfg(target_os = "linux")]
mod native;

use std::mem::replace;

use bientot_domain::accounting::SessionState;
use bientot_platform::PowerState;
use bientot_platform::lifecycle::{LifecycleChange, LockReporting, SessionSnapshot};

#[cfg(target_os = "linux")]
pub use native::LogindSource;

/// A signal from logind or from the session's screensaver, already decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// `PropertiesChanged` of the own session: the properties it carries.
    SessionChanged {
        locked_hint: Option<bool>,
        active: Option<bool>,
    },
    /// `ActiveChanged` of `org.freedesktop.ScreenSaver` or `org.gnome.ScreenSaver`.
    ScreenSaverActive(bool),
    PrepareForSleep(bool),
}

/// The session as read once, when the observation starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionReadings {
    pub locked_hint: bool,
    pub active: bool,
    /// The screensaver's `GetActive`; `None` when no screensaver answers.
    pub screensaver: Option<bool>,
}

/// Which signal tells whether the screen is locked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LockSignal {
    /// Nothing does: only a switch of the session tells.
    Unreported,
    /// The screensaver, until `LockedHint` is first seen true.
    ScreenSaver { active: bool },
    /// `LockedHint`, once seen true: from then on the desktop is known to set it.
    LockedHint { locked: bool },
}

impl LockSignal {
    fn with_locked_hint(self, locked: bool) -> Self {
        if locked || matches!(self, Self::LockedHint { .. }) {
            Self::LockedHint { locked }
        } else {
            self
        }
    }

    fn with_screensaver(self, active: bool) -> Self {
        match self {
            Self::LockedHint { .. } => self,
            Self::Unreported | Self::ScreenSaver { .. } => Self::ScreenSaver { active },
        }
    }

    fn locked(self) -> Option<bool> {
        match self {
            Self::Unreported => None,
            Self::ScreenSaver { active } => Some(active),
            Self::LockedHint { locked } => Some(locked),
        }
    }
}

/// The session and the power of one logind session, as the signals leave them.
///
/// A session is locked while it is not the active one, as Windows records Switch User as a
/// lock, or while its lock signal says so. A change is reported only when a signal changes what
/// is known and the session or the power ends up in another state than the one last reported:
/// a signal repeated, or the lock screen announced by two sources, records one change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTracker {
    active: bool,
    lock: LockSignal,
    power: PowerState,
    reported: SessionState,
}

impl SessionTracker {
    /// Starts from the session as read: the state the snapshot gives is the one reported.
    pub fn new(readings: SessionReadings) -> Self {
        let lock = LockSignal::Unreported.with_locked_hint(readings.locked_hint);
        let lock = readings
            .screensaver
            .map_or(lock, |active| lock.with_screensaver(active));
        let mut tracker = Self {
            active: readings.active,
            lock,
            power: PowerState::Active,
            reported: SessionState::Unknown,
        };
        tracker.reported = tracker.snapshot().session;
        tracker
    }

    /// The session now. Without a lock signal the lock state is unknown, even while the
    /// session is switched away: the user is told that this desktop does not report locks.
    pub fn snapshot(&self) -> SessionSnapshot {
        match self.lock {
            LockSignal::Unreported => SessionSnapshot {
                session: SessionState::Unknown,
                lock_reporting: LockReporting::Unavailable,
            },
            lock => SessionSnapshot {
                session: session_state(self.active, lock),
                lock_reporting: LockReporting::Live,
            },
        }
    }

    pub fn apply(&mut self, signal: Signal) -> Option<LifecycleChange> {
        match signal {
            Signal::SessionChanged {
                locked_hint,
                active,
            } => {
                let lock =
                    locked_hint.map_or(self.lock, |locked| self.lock.with_locked_hint(locked));
                self.update_session(active.unwrap_or(self.active), lock)
            }
            Signal::ScreenSaverActive(active) => {
                self.update_session(self.active, self.lock.with_screensaver(active))
            }
            Signal::PrepareForSleep(sleeping) => {
                let power = if sleeping {
                    PowerState::Suspended
                } else {
                    PowerState::Active
                };
                (replace(&mut self.power, power) != power).then_some(LifecycleChange::Power(power))
            }
        }
    }

    fn update_session(&mut self, active: bool, lock: LockSignal) -> Option<LifecycleChange> {
        if (active, lock) == (self.active, self.lock) {
            return None;
        }
        self.active = active;
        self.lock = lock;
        let session = session_state(active, lock);
        (replace(&mut self.reported, session) != session)
            .then_some(LifecycleChange::Session(session))
    }
}

fn session_state(active: bool, lock: LockSignal) -> SessionState {
    if !active || lock.locked() == Some(true) {
        SessionState::Locked
    } else {
        SessionState::Unlocked
    }
}

#[cfg(test)]
mod tests;
