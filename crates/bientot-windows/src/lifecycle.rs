//! Session lock and unlock, suspend and resume, ported from `Lifecycle/WindowsLifecyclePorts.cs`,
//! `Sessions/SessionMessageDecoder.cs`, `Sessions/WtsSessionStateProbe.cs` and
//! `Power/PowerMessageDecoder.cs`.
//!
//! Windows reports them as window messages. The .NET app subclasses its main window; GPUI owns
//! the Rust app's window procedure, so on Windows [`LifecycleWindow`] opens a message-only
//! window of its own on its own thread and registers it for both notifications. The decoding and
//! the stamping below are platform independent.

#[cfg(windows)]
mod native;

use bientot_domain::accounting::SessionState;
use bientot_domain::time::{Clock, TimeZoneProvider};
use bientot_platform::PowerState;
use bientot_platform::lifecycle::{LifecycleChange, LifecycleTransition};

#[cfg(windows)]
pub use native::{LifecycleWindow, current_session_state};

const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WTS_SESSION_LOCK: usize = 0x7;
const WTS_SESSION_UNLOCK: usize = 0x8;
const WM_POWERBROADCAST: u32 = 0x0218;
const PBT_APMSUSPEND: usize = 0x4;
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;
const WTS_SESSIONSTATE_LOCK: i32 = 0;
const WTS_SESSIONSTATE_UNLOCK: i32 = 1;

/// A message as a window procedure receives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowMessage {
    pub message: u32,
    pub w_param: usize,
    pub l_param: isize,
}

/// The lock or unlock of `current_session_id`, or a suspend or automatic resume. Every other
/// message is ignored, including another session's changes and the user resume
/// (`PBT_APMRESUMESUSPEND`) that follows an automatic resume, so that one resume is recorded.
pub fn decode(message: WindowMessage, current_session_id: u32) -> Option<LifecycleChange> {
    match (message.message, message.w_param) {
        (WM_WTSSESSION_CHANGE, _) if message.l_param as u32 != current_session_id => None,
        (WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK) => {
            Some(LifecycleChange::Session(SessionState::Locked))
        }
        (WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK) => {
            Some(LifecycleChange::Session(SessionState::Unlocked))
        }
        (WM_POWERBROADCAST, PBT_APMSUSPEND) => Some(LifecycleChange::Power(PowerState::Suspended)),
        (WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC) => {
            Some(LifecycleChange::Power(PowerState::Active))
        }
        _ => None,
    }
}

/// The session state for the `SessionFlags` of `WTSINFOEX_LEVEL1`, or `None` when the query
/// failed.
pub fn session_state(session_flags: Option<i32>) -> SessionState {
    match session_flags {
        Some(WTS_SESSIONSTATE_LOCK) => SessionState::Locked,
        Some(WTS_SESSIONSTATE_UNLOCK) => SessionState::Unlocked,
        _ => SessionState::Unknown,
    }
}

/// Turns the window messages of one session into transitions, reading the clock and the local
/// time zone once for each change and not at all for an ignored message.
pub struct LifecycleMessages<C, Z> {
    current_session_id: u32,
    clock: C,
    zones: Z,
}

impl<C: Clock, Z: TimeZoneProvider> LifecycleMessages<C, Z> {
    pub fn new(current_session_id: u32, clock: C, zones: Z) -> Self {
        Self {
            current_session_id,
            clock,
            zones,
        }
    }

    pub fn transition(&self, message: WindowMessage) -> Option<LifecycleTransition> {
        let change = decode(message, self.current_session_id)?;
        Some(LifecycleTransition {
            change,
            occurred_at: self.clock.now(),
            time_zone_id: self.zones.local().0,
        })
    }
}

#[cfg(test)]
mod tests;
