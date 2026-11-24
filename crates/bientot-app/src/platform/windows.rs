//! The Windows backend: the ports of `bientot-windows`, the toasts, the « Animation effects »
//! setting, the notification-area icon and the flyout's corner, Velopack's hooks and the message
//! box of last resort.

#[path = "windows/tray.rs"]
mod tray;

use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Context as _;
use bientot_domain::accounting::SESSION_HISTORY_SOURCE_ID;
use bientot_domain::time::SystemClock;
use bientot_platform::lifecycle::{
    LifecycleListener, LifecycleSource, LockReporting, SessionSnapshot,
};
use bientot_platform::motion::{OnChange, ReducedMotion};
use bientot_platform::notifications::QueuedClicks;
use bientot_platform::power_history::HistoryUnavailable;
use bientot_platform::session_history::{LockHistory, SessionBoundaries, SessionBoundaryLog};
use bientot_platform::startup::StartupPort;
use bientot_windows::activity::SystemLastInput;
use bientot_windows::animation_effects::{self, SettingChanges};
use bientot_windows::lifecycle::{LifecycleWindow, current_session_state};
use bientot_windows::notifications::{Sender, Toasts};
use bientot_windows::power_history::system_log;
use bientot_windows::session_history::current_user_log;
use bientot_windows::startup::{StartupRegistration, current_activation, launch_intent};
use jiff::Timestamp;

use super::{Platform, PlatformContext};
use crate::live::LiveHost;
use crate::notify::ToastSink;
use crate::strings::Language;
use crate::zones::SystemZones;

#[cfg(feature = "updates")]
pub use crate::updates::direct::{
    install_pending as install_pending_update, run_hooks as run_update_hooks,
};
pub use bientot_windows::process::wait_for_exit;
pub use bientot_windows::single_instance::PrimaryInstance as Instance;
pub use tray::{place_window, tray_window};

pub fn build(ctx: &PlatformContext<'_>) -> anyhow::Result<Platform> {
    let PlatformContext { directory, launch } = *ctx;
    let demo = launch.demo.is_some();
    // Before any window, the onboarding's included, as `Toasts::register` requires. A demo shows
    // no toasts, as in .NET. A click queues until the day takes it, so the one COM started this
    // process for waits for the day, after the onboarding on a first run.
    let clicks = QueuedClicks::default();
    let toasts = (!demo).then(|| {
        let data_dir = launch.data_dir.as_ref().map(|_| directory.path());
        register_toasts(data_dir, clicks.clone())
    });
    let launch_intent = launch_intent(launch.windows_startup, &current_activation());
    // A demo leaves the user's registration alone, as in .NET.
    let startup = (!demo).then(|| {
        let registration = StartupRegistration::for_current_process();
        registration.migrate_owned_legacy_registration();
        Arc::new(registration) as Arc<dyn StartupPort>
    });
    let (tray, tray_events) = tray::NotificationArea::new();
    Ok(Platform {
        launch_intent,
        last_input: Box::new(SystemLastInput),
        lifecycle: Arc::new(SessionWindow),
        power_log: Box::new(system_log()),
        session_log: Box::new(CurrentUserLog),
        notifications: Box::new(move |host, language| {
            if let Some(toasts) = toasts {
                enable_toasts(toasts, host, language);
            }
        }),
        notification_clicks: Box::new(clicks),
        startup,
        tray: Rc::new(tray),
        tray_events,
        desktop: Rc::new(tray::TaskbarCorner),
    })
}

/// Registers the toasts of this process, which a click starts on `data_dir` when given.
fn register_toasts(data_dir: Option<&Path>, clicks: QueuedClicks) -> windows::core::Result<Toasts> {
    Sender::current(data_dir)
        .and_then(|sender| Toasts::register(sender, move |delivery_id| clicks.click(delivery_id)))
}

/// Has the host show its reminders through the toasts, as `App.xaml.cs` does outside a demo.
///
/// Windows shows a toast only for a sender it knows: the MSIX package, or the AppUserModelID the
/// portable build registers. A failed registration shows no reminder, so that none is reserved in
/// the ledger without being shown, and the flyout says so.
fn enable_toasts(
    toasts: windows::core::Result<Toasts>,
    host: &mut LiveHost<'static>,
    language: Language,
) {
    match toasts {
        Ok(toasts) => host.enable_notifications(Box::leak(Box::new(ToastSink {
            toasts,
            language,
            zones: &SystemZones,
        }))),
        Err(error) => host.notifications_refused(&error),
    }
}

/// The session and power changes of this session, from a message-only window of its own thread,
/// which receives them before GPUI runs.
struct SessionWindow;

impl LifecycleSource for SessionWindow {
    fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            session: current_session_state(),
            lock_reporting: LockReporting::Live,
        }
    }

    fn watch(&self, mut listener: Box<dyn LifecycleListener>) -> io::Result<Box<dyn Send>> {
        let window = LifecycleWindow::open(SystemClock, SystemZones, move |transition| {
            listener.changed(transition);
        })?;
        Ok(Box::new(window))
    }
}

/// The Winlogon log of the user running this process, who is looked up as the log is read.
struct CurrentUserLog;

impl SessionBoundaryLog for CurrentUserLog {
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
        match current_user_log() {
            Some(log) => log.read(since, record_limit),
            None => Err(HistoryUnavailable::NoSource),
        }
    }
}

/// The « Animation effects » setting.
pub struct Motion;

pub fn motion() -> Motion {
    Motion
}

impl ReducedMotion for Motion {
    type Watch = SettingChanges;

    fn current(&self) -> bool {
        animation_effects::reduce_motion()
    }

    /// The hook runs inside a window procedure, where GPUI may hold the app: `on_change` should
    /// only queue.
    fn watch(&self, on_change: OnChange) -> io::Result<SettingChanges> {
        SettingChanges::watch(on_change)
    }
}

#[cfg(not(feature = "updates"))]
pub fn run_update_hooks() -> bool {
    false
}

#[cfg(not(feature = "updates"))]
pub fn install_pending_update() {}

pub fn local_app_data() -> anyhow::Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .context("no folder for the application data")
}

/// Release builds have no console: a message box tells it.
pub fn show_fatal(title: &str, body: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW};
    use windows::core::{HSTRING, PCWSTR};
    let (text, title) = (HSTRING::from(body), HSTRING::from(title));
    // SAFETY: both strings outlive the modal call.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONWARNING,
        )
    };
}
