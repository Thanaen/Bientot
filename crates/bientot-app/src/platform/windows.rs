//! The Windows backend: the ports of `bientot-windows`, the toasts, the « Animation effects »
//! setting, the notification-area icon and the flyout's corner, the Store and Velopack's updates,
//! the common item dialogs, the time zone's Windows key, the final paths, and the message box of
//! last resort.

#[cfg(feature = "updates")]
#[path = "windows/direct.rs"]
mod direct;
#[path = "windows/tray.rs"]
mod tray;

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Context as _;
use bientot_core::diagnostics::FinalPaths;
use bientot_domain::accounting::SESSION_HISTORY_SOURCE_ID;
use bientot_domain::time::SystemClock;
use bientot_platform::lifecycle::{
    LifecycleListener, LifecycleSource, LockReporting, SessionSnapshot,
};
use bientot_platform::motion::{OnChange, ReducedMotion};
use bientot_platform::notifications::{Notifier, QueuedClicks};
use bientot_platform::power_history::HistoryUnavailable;
use bientot_platform::session_history::{LockHistory, SessionBoundaries, SessionBoundaryLog};
use bientot_platform::startup::StartupPort;
use bientot_windows::activity::SystemLastInput;
use bientot_windows::animation_effects::{self, SettingChanges};
use bientot_windows::diagnostics::{directory_is_final, file_is_in};
use bientot_windows::lifecycle::{LifecycleWindow, current_session_state};
use bientot_windows::notifications::{Sender, Toasts};
use bientot_windows::power_history::system_log;
use bientot_windows::session_history::current_user_log;
use bientot_windows::startup::{StartupRegistration, current_activation, launch_intent};
use bientot_windows::updates::StoreUpdates;
use gpui::{App, AsyncApp};
use jiff::Timestamp;

use super::{Platform, PlatformContext};
use crate::updates::StoreError;
use crate::zones::SystemZones;

pub use bientot_windows::diagnostics::{final_directory, native_machine, operating_system_version};
pub use bientot_windows::process::wait_for_exit;
pub use bientot_windows::single_instance::PrimaryInstance as Instance;
#[cfg(feature = "updates")]
pub use direct::{install_pending as install_pending_update, run_hooks as run_update_hooks};
pub use tray::place_window;

/// Windows shows a toast only for a sender it knows: the MSIX package, or the AppUserModelID
/// the portable build registers. A failed registration shows no reminder, so that none is
/// reserved in the ledger without being shown, and the flyout says so.
pub type NotificationsRefused = windows::core::Error;

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
    let launch_intent = launch_intent(launch.sign_in, &current_activation());
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
        notifier: toasts.map(|toasts| toasts.map(|toasts| Box::new(toasts) as Box<dyn Notifier>)),
        notification_clicks: Box::new(clicks),
        startup,
        tray: Rc::new(tray),
        tray_events,
        tray_backend: None,
        desktop: Rc::new(tray::TaskbarCorner),
    })
}

/// Registers the toasts of this process, which a click starts on `data_dir` when given.
fn register_toasts(data_dir: Option<&Path>, clicks: QueuedClicks) -> windows::core::Result<Toasts> {
    Sender::current(data_dir)
        .and_then(|sender| Toasts::register(sender, move |delivery_id| clicks.click(delivery_id)))
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

pub fn activate_window(window: &mut gpui::Window) {
    window.activate_window();
}

#[cfg(not(feature = "updates"))]
pub fn run_update_hooks() -> bool {
    false
}

#[cfg(not(feature = "updates"))]
pub fn install_pending_update() {}

/// The feed of the build Velopack's `Setup.exe` installed. `None` when Velopack did not install
/// the running exe, as for `cargo run`.
#[cfg(feature = "updates")]
pub fn direct_updates() -> Option<direct::DirectUpdates> {
    direct::DirectUpdates::installed()
}

/// Only the `Setup.exe` build carries Velopack.
#[cfg(not(feature = "updates"))]
pub fn direct_updates() -> Option<crate::updates::NoUpdates> {
    None
}

/// The Microsoft Store, whose prompts the notification-area icon's window owns: the flyout
/// closes when Windows' prompt takes the focus. `None` before the icon shows, and for a build
/// without package identity, which has no Store to ask.
pub fn store_updates() -> windows::core::Result<Option<StoreUpdates>> {
    let Some(owner) = tray::tray_window() else {
        return Ok(None);
    };
    StoreUpdates::for_window(owner)
}

impl StoreError for windows::core::Error {
    fn hresult(&self) -> Option<i32> {
        Some(self.code().0)
    }
}

pub async fn pick_save(
    suggested: String,
    file_type: String,
    extension: &'static str,
    _cx: &mut AsyncApp,
) -> anyhow::Result<Option<PathBuf>> {
    off_thread(move || bientot_windows::dialogs::pick_save_file(&suggested, &file_type, extension))
        .await
}

pub async fn pick_open(
    file_type: String,
    extensions: &'static [&'static str],
    _cx: &mut AsyncApp,
) -> anyhow::Result<Option<PathBuf>> {
    off_thread(move || bientot_windows::dialogs::pick_open_file(&file_type, extensions)).await
}

/// The common item dialog blocks until closed: it runs on a thread of its own.
async fn off_thread(
    dialog: impl FnOnce() -> windows::core::Result<Option<PathBuf>> + Send + 'static,
) -> anyhow::Result<Option<PathBuf>> {
    let (picked, chosen) = futures::channel::oneshot::channel();
    std::thread::Builder::new()
        .name("worktime-file-dialog".to_owned())
        .spawn(move || {
            picked.send(dialog()).ok();
        })?;
    Ok(chosen.await??)
}

pub fn open_in_editor(file: &Path, cx: &App) {
    cx.open_with_system(file);
}

/// The key of the zone in Windows' registry, such as `Romance Standard Time`.
pub fn system_zone_id() -> Option<String> {
    use windows::Win32::System::Time::{
        DYNAMIC_TIME_ZONE_INFORMATION, GetDynamicTimeZoneInformation, TIME_ZONE_ID_INVALID,
    };
    let mut information = DYNAMIC_TIME_ZONE_INFORMATION::default();
    // SAFETY: the out pointer is a local for the call.
    let result = unsafe { GetDynamicTimeZoneInformation(&mut information) };
    let key = &information.TimeZoneKeyName;
    let length = key.iter().position(|unit| *unit == 0).unwrap_or(key.len());
    if result == TIME_ZONE_ID_INVALID || length == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&key[..length]))
}

/// Asks Windows where the log directory and each log file really are.
struct WindowsFinalPaths;

impl FinalPaths for WindowsFinalPaths {
    fn directory_is_final(&self, directory: &Path) -> bool {
        directory_is_final(directory)
    }

    fn file_is_in(&self, file: &File, directory: &Path) -> bool {
        file_is_in(file, directory)
    }
}

pub fn final_paths() -> &'static (dyn FinalPaths + Sync) {
    &WindowsFinalPaths
}

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
