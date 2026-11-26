//! How long the user has not touched the keyboard or the pointer (research.md, R6;
//! contracts/linux-backends.md, « Idle »).
//!
//! No single interface tells it on Linux. Each session type tries its backends in a fixed order
//! and keeps the first that answers; [`IdleProbe`] turns that backend's « idle for d » into the
//! reading Windows takes from `GetLastInputInfo`, so that `BuiltInActivitySource` and the
//! domain's tracker stay as they are.

pub mod ext_idle;
pub mod screensaver;

#[cfg(target_os = "linux")]
mod mutter;
#[cfg(target_os = "linux")]
mod native;
#[cfg(target_os = "linux")]
mod xss;

#[cfg(target_os = "linux")]
pub use native::{Monotonic, SystemBackends};

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use bientot_domain::sources::LastInputReading;
use bientot_platform::activity::{LastInputProbe, reading_from_idle};
use bientot_platform::system::SessionType;

/// A way to read the idle time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Backend {
    /// `ext_idle_notifier_v1`, on a Wayland connection of its own.
    ExtIdleNotify,
    MutterIdleMonitor,
    /// Cinnamon's Mutter fork, with the same interface under another name.
    MuffinIdleMonitor,
    /// `org.freedesktop.ScreenSaver.GetSessionIdleTime`, in the unit it was found to use.
    ScreenSaver,
    /// The X11 `MIT-SCREEN-SAVER` extension. Under XWayland it only sees the input sent to X
    /// clients, so a Wayland session never uses it.
    XScreenSaver,
}

impl Backend {
    /// The name the diagnostic log and export give it.
    pub fn name(self) -> &'static str {
        match self {
            Self::ExtIdleNotify => "ext-idle-notify-v1",
            Self::MutterIdleMonitor => "org.gnome.Mutter.IdleMonitor",
            Self::MuffinIdleMonitor => "org.cinnamon.Muffin.IdleMonitor",
            Self::ScreenSaver => "org.freedesktop.ScreenSaver",
            Self::XScreenSaver => "MIT-SCREEN-SAVER",
        }
    }
}

/// The backends a session tries, first to last. Without a display server there are none.
pub fn order(session: Option<SessionType>) -> &'static [Backend] {
    use Backend::*;
    match session {
        Some(SessionType::Wayland) => &[ExtIdleNotify, MutterIdleMonitor, ScreenSaver],
        Some(SessionType::X11) => &[XScreenSaver, MutterIdleMonitor, MuffinIdleMonitor],
        None => &[],
    }
}

/// The first backend of the session's order that `open` opens, with what it opened. The
/// backends after it are not tried.
pub fn select<T>(
    session: Option<SessionType>,
    mut open: impl FnMut(Backend) -> Option<T>,
) -> Option<(Backend, T)> {
    order(session)
        .iter()
        .find_map(|&backend| open(backend).map(|opened| (backend, opened)))
}

/// An open backend.
pub trait IdleReader: Send {
    /// How long since the last input at the monotonic instant `now`, or `None` once the backend
    /// stopped answering (its bus or compositor went away).
    fn idle(&mut self, now: Duration) -> Option<Duration>;
}

/// A backend as it opened.
pub struct Opened {
    pub reader: Box<dyn IdleReader>,
    /// False when an app that inhibits idle, such as a video call, holds the idle time at zero:
    /// version 1 of `ext_idle_notifier_v1`.
    pub inhibitor_blind: bool,
}

/// The backends of a machine.
pub trait Backends: Send {
    /// Opens `backend` when its probe passes.
    fn open(&mut self, backend: Backend) -> Option<Opened>;
}

/// `CLOCK_MONOTONIC`: the time since the boot, sleep excluded.
pub trait MonotonicClock: Send {
    fn now(&self) -> Duration;
}

/// What the diagnostic log says about the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleNote {
    /// `idle.backend=<name>`, `none` when no backend answers: the first selection.
    Selected(Option<Backend>),
    /// `idle.backend_changed`: a later selection chose another backend, or none.
    Changed {
        from: Option<Backend>,
        to: Option<Backend>,
    },
    /// `idle.inhibitor_blind=false`: the backend just chosen counts an app that inhibits idle as
    /// activity.
    InhibitorsCount,
}

/// The last-input probe of a Linux session: the idle time of the selected backend at a
/// monotonic instant, as [`reading_from_idle`] gives it.
///
/// The selection runs when the probe is made, again at once when the backend in use stops
/// answering, and every [`RETRY`] while none answers. It may take a second or more (the
/// screensaver's unit check), so the probe is best made and read off the UI thread.
pub struct IdleProbe<B, C> {
    session: Option<SessionType>,
    clock: C,
    selection: Mutex<Selection<B>>,
    log: Box<dyn Fn(IdleNote) + Send>,
}

struct Selection<B> {
    backends: B,
    chosen: Option<(Backend, Box<dyn IdleReader>)>,
    /// What the log last said was chosen; `None` before the first selection.
    logged: Option<Option<Backend>>,
    next_attempt: Duration,
}

/// How long a session without a backend waits before trying them all again.
pub const RETRY: Duration = Duration::from_secs(60);

impl<B: Backends, C: MonotonicClock> IdleProbe<B, C> {
    /// Selects the first backend of `session` that answers, and logs it with `log`.
    pub fn new(
        session: Option<SessionType>,
        backends: B,
        clock: C,
        log: impl Fn(IdleNote) + Send + 'static,
    ) -> Self {
        let probe = Self {
            session,
            clock,
            selection: Mutex::new(Selection {
                backends,
                chosen: None,
                logged: None,
                next_attempt: Duration::ZERO,
            }),
            log: Box::new(log),
        };
        let now = probe.clock.now();
        probe.lock().select(probe.session, now, &probe.log);
        probe
    }

    /// The backend in use, for the diagnostic export.
    pub fn backend(&self) -> Option<Backend> {
        self.lock().chosen.as_ref().map(|(backend, _)| *backend)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Selection<B>> {
        self.selection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl<B: Backends> Selection<B> {
    fn select(&mut self, session: Option<SessionType>, now: Duration, log: &dyn Fn(IdleNote)) {
        let chosen = select(session, |backend| self.backends.open(backend));
        let backend = chosen.as_ref().map(|(backend, _)| *backend);
        let note = match self.logged.replace(backend) {
            None => Some(IdleNote::Selected(backend)),
            Some(from) if from != backend => Some(IdleNote::Changed { from, to: backend }),
            Some(_) => None,
        };
        if let Some(note) = note {
            log(note);
            if chosen
                .as_ref()
                .is_some_and(|(_, opened)| !opened.inhibitor_blind)
            {
                log(IdleNote::InhibitorsCount);
            }
        }
        self.chosen = chosen.map(|(backend, opened)| (backend, opened.reader));
        self.next_attempt = now + RETRY;
    }

    fn idle(&mut self, now: Duration) -> Option<Duration> {
        let (_, reader) = self.chosen.as_mut()?;
        let idle = reader.idle(now);
        if idle.is_none() {
            self.chosen = None;
            self.next_attempt = now;
        }
        idle
    }
}

impl<B: Backends, C: MonotonicClock> LastInputProbe for IdleProbe<B, C> {
    fn read(&self) -> Option<LastInputReading> {
        let mut selection = self.lock();
        let now = self.clock.now();
        let idle = selection.idle(now).or_else(|| {
            if now < selection.next_attempt {
                return None;
            }
            selection.select(self.session, now, &self.log);
            selection.idle(now)
        })?;
        Some(reading_from_idle(now, idle))
    }
}

#[cfg(test)]
mod tests;
