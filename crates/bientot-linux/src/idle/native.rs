//! The backends of this machine and its monotonic clock.

use std::time::Duration;

use bientot_platform::system::SessionType;
use rustix::time::{ClockId, clock_gettime};

use super::{
    Backend, Backends, IdleNote, IdleReader, IdleWatch, MonotonicClock, Opened, ext_idle, mutter,
    screensaver, xss,
};

/// `CLOCK_MONOTONIC`, the clock the compositors and X servers date input with.
#[derive(Clone, Copy, Debug, Default)]
pub struct Monotonic;

impl MonotonicClock for Monotonic {
    fn now(&self) -> Duration {
        let now = clock_gettime(ClockId::Monotonic);
        Duration::new(now.tv_sec as u64, now.tv_nsec as u32)
    }
}

/// Each backend on the buses and display servers of this process's environment.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemBackends;

impl Backends for SystemBackends {
    fn open(&mut self, backend: Backend) -> Option<Opened> {
        let reader: Box<dyn IdleReader> = match backend {
            Backend::ExtIdleNotify => return ext_idle::open(),
            Backend::MutterIdleMonitor => Box::new(mutter::open(mutter::MUTTER)?),
            Backend::MuffinIdleMonitor => Box::new(mutter::open(mutter::MUFFIN)?),
            Backend::ScreenSaver => Box::new(screensaver::open()?),
            Backend::XScreenSaver => Box::new(xss::open()?),
        };
        Some(Opened {
            reader,
            inhibitor_blind: true,
        })
    }
}

impl IdleWatch<Monotonic> {
    /// The watch of the session this process runs in.
    pub fn of_this_session(log: impl Fn(IdleNote) + Send + 'static) -> std::io::Result<Self> {
        Self::new(this_session(), SystemBackends, Monotonic, log)
    }
}

fn this_session() -> Option<SessionType> {
    crate::system::session_type(|name| std::env::var_os(name))
}

#[cfg(test)]
mod tests;
