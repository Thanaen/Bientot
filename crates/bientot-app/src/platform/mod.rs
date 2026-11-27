//! The composition root of the system integration. [`build`] makes, once per launch, the ports
//! the app runs on, from the backend of the system it was compiled for. The rest of the app takes
//! the ports and names no system: the platform conditions of the app belong in this module.
//!
//! What a launch needs before it holds its data folder is no part of [`Platform`]: the folder's
//! [`Instance`], the [`motion`] setting the startup failure window follows, and the functions
//! below.

#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(not(any(windows, target_os = "linux")), path = "null.rs")]
mod backend;

use std::rc::Rc;
use std::sync::Arc;

use bientot_core::diagnostics::{DiagnosticLogLevel, DiagnosticValue};
use bientot_domain::sources::LastInputReading;
use bientot_platform::activity::LastInputProbe;
use bientot_platform::launcher::LauncherPort;
use bientot_platform::lifecycle::{InMemoryLifecycle, LifecycleSource};
use bientot_platform::notifications::{NotificationClicks, Notifier, QueuedClicks};
use bientot_platform::placement::{Desktop, FlyoutPlacement, InMemoryDesktop};
use bientot_platform::power_history::{HistoryUnavailable, PowerBoundaries, PowerBoundaryLog};
use bientot_platform::session_history::{LockHistory, SessionBoundaries, SessionBoundaryLog};
use bientot_platform::startup::{LaunchIntent, StartupPort};
use bientot_platform::tray::{InMemoryTray, Tray, TrayEvent, TrayHost};
use bientot_store::data_directory::DataDirectory;
use futures::channel::mpsc;
use jiff::Timestamp;

use crate::launch::Launch;
use crate::strings::Wording;
use crate::tray_model::MenuCommand;

/// Whose words the texts take, unless the screenshots choose another system's.
#[cfg(target_os = "linux")]
pub const WORDING: Wording = Wording::Linux;
#[cfg(not(target_os = "linux"))]
pub const WORDING: Wording = Wording::Windows;

/// Holds the data folder: a later launch on it shows this one.
pub use backend::Instance;
/// Why the system refused to show this app's notifications.
pub use backend::NotificationsRefused;
/// Brings a window of the app to the front with the keyboard, which a window manager does not
/// give every window GPUI opens.
pub use backend::activate_window;
pub use backend::build;
/// Where a folder Bientôt created really is, for the file manager to open: under MSIX, in the
/// package's `LocalCache`. `None` when it is where its path says.
pub use backend::final_directory;
/// Where the user's data really is, once links and mount points are followed: the diagnostic
/// export checks its log against it.
pub use backend::final_paths;
/// Installs the update a check downloaded, if any, and ends the process.
pub use backend::install_pending_update;
/// The folder that holds the user's data folder: `%LOCALAPPDATA%`, or the XDG data folder.
pub use backend::local_app_data;
/// The machine the system runs on, as an `IMAGE_FILE_MACHINE`: `None` when the system does not
/// tell, and the process's own architecture then stands for it.
pub use backend::native_machine;
/// Opens a file of the user's in the editor the system associates with its type.
pub use backend::open_in_editor;
/// The four parts of the system's version, as .NET's `Environment.OSVersion` gives them; zeros
/// when the system does not tell.
pub use backend::operating_system_version;
/// Moves the open flyout's window so that its client area covers the bounds given, in physical
/// pixels: GPUI only resizes a window, and `bientot-platform` cannot name it.
pub use backend::place_window;
/// Runs the hooks of the installer before anything else; whether this launch may install a
/// pending update.
pub use backend::run_update_hooks;
/// Tells the user why Bientôt cannot start when no window can open.
pub use backend::show_fatal;
/// The system as « À propos » and the diagnostic export describe it: `None` where they describe
/// none, as on Windows, whose export keeps the one .NET wrote.
pub use backend::system;
/// The languages the user set the system to, which [`crate::strings::LanguageSources`] reads:
/// the display languages of Windows, the POSIX locale of the messages on Linux.
pub use backend::system_locale;
/// The system's name for its own time zone when it is not an IANA name: the Windows key, which
/// the ledger keeps as .NET's `TimeZoneInfo.Local.Id`.
pub use backend::system_zone_id;
/// Waits up to a timeout for a process to exit; whether it did.
pub use backend::wait_for_exit;
/// Whether the user asked the system for less motion: [`motion`] reads it.
pub use backend::{Motion, motion};
/// The updates of the build Velopack installed (`Setup.exe`'s, or the AppImage), and of the
/// Microsoft Store, whose prompts the notification-area icon's window owns: `None` when this build
/// has none.
pub use backend::{direct_updates, store_updates};
/// The save and open dialogs: `None` once the user cancels. On Windows the dialog blocks until
/// closed, so it runs on a thread of its own; elsewhere GPUI's own dialogs keep the flow.
pub use backend::{pick_open, pick_save};

/// The TLS of every request Bientôt makes, trusting the system's root certificates: SChannel on
/// Windows, rustls elsewhere (`Cargo.toml` turns on each one's ureq feature).
#[cfg(any(feature = "usage-ping", feature = "updates"))]
pub fn system_tls() -> ureq::tls::TlsConfig {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

    TlsConfig::builder()
        .provider(if cfg!(windows) {
            TlsProvider::NativeTls
        } else {
            TlsProvider::Rustls
        })
        .root_certs(RootCerts::PlatformVerifier)
        .build()
}

/// What [`build`] knows of the launch.
pub struct PlatformContext<'a> {
    /// The data folder this launch holds.
    pub directory: &'a DataDirectory,
    pub launch: &'a Launch,
}

/// The clicks on the tray icon and the menu items chosen, in the order they came.
pub type TrayEvents = mpsc::UnboundedReceiver<TrayEvent>;

/// The lines the backends write to the diagnostic log once the day opens, in the order they
/// came. Closed where no backend writes any.
pub type BackendNotes = mpsc::UnboundedReceiver<BackendNote>;

/// A line of the diagnostic log from a backend, such as the idle backend chosen on Linux.
#[derive(Clone, Debug, PartialEq)]
pub struct BackendNote {
    pub level: DiagnosticLogLevel,
    pub event: &'static str,
    pub properties: Vec<(&'static str, DiagnosticValue<'static>)>,
}

/// The ports of the system, for one launch.
pub struct Platform {
    /// How this launch started, by the user or at sign-in.
    pub launch_intent: LaunchIntent,
    pub last_input: Box<dyn LastInputProbe + Send>,
    pub lifecycle: Arc<dyn LifecycleSource + Send + Sync>,
    pub power_log: Box<dyn PowerBoundaryLog + Send>,
    pub session_log: Box<dyn SessionBoundaryLog + Send>,
    /// What shows the live day's reminders, or why the system refused it. `None` in a demo, and
    /// where the system shows no notifications.
    pub notifier: Option<Result<Box<dyn Notifier>, NotificationsRefused>>,
    pub notification_clicks: Box<dyn NotificationClicks>,
    /// `None` in a demo, which leaves the user's registration alone, and where the system cannot
    /// start the app at sign-in.
    pub startup: Option<Arc<dyn StartupPort>>,
    /// This copy's entry in the desktop's menu of applications, where the copy writes its own:
    /// the AppImage. `None` in a demo, and for every other install.
    pub launcher: Option<Arc<dyn LauncherPort>>,
    /// Shows nothing until the app first publishes the day to it.
    pub tray: Rc<dyn Tray<MenuCommand>>,
    pub tray_events: TrayEvents,
    pub backends: Backends,
    pub backend_notes: BackendNotes,
    /// Where the flyout opens.
    pub desktop: Rc<dyn Desktop>,
}

impl Platform {
    /// Ports that answer « unknown » or do nothing: tests, screenshots, other systems.
    pub fn null() -> Self {
        Self {
            launch_intent: LaunchIntent::Manual,
            last_input: Box::new(Unknown),
            lifecycle: Arc::new(InMemoryLifecycle::default()),
            power_log: Box::new(Unknown),
            session_log: Box::new(Unknown),
            notifier: None,
            notification_clicks: Box::new(QueuedClicks::default()),
            startup: None,
            launcher: None,
            tray: Rc::new(InMemoryTray::new(Vec::new(), None, TrayHost::Absent)),
            tray_events: mpsc::unbounded().1,
            backends: Backends::default(),
            backend_notes: mpsc::unbounded().1,
            desktop: Rc::new(InMemoryDesktop {
                placement: FlyoutPlacement::CompositorPlaced,
                usable_height: None,
                system_draws_frame: false,
            }),
        }
    }
}

/// The name of the backend in use for each concern, which the diagnostic export gives: `None`
/// where it names none, as on Windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Backends {
    /// Also in the diagnostic log, with whether a host shows the icon.
    pub tray: Option<&'static str>,
    pub notifications: Option<&'static str>,
    pub autostart: Option<&'static str>,
    /// The idle time's, as chosen at start: the diagnostic log tells each later change.
    pub idle: Option<&'static str>,
    /// The lock, unlock, sleep and resume's.
    pub lifecycle: Option<&'static str>,
}

impl Backends {
    /// Each concern with its backend's name, as the export writes them.
    pub fn named(self) -> Vec<(&'static str, &'static str)> {
        [
            ("tray", self.tray),
            ("notifications", self.notifications),
            ("autostart", self.autostart),
            ("idle", self.idle),
            ("lifecycle", self.lifecycle),
        ]
        .into_iter()
        .filter_map(|(concern, backend)| Some((concern, backend?)))
        .collect()
    }
}

impl gpui::Global for Backends {}

/// A system that tells nothing of the last input nor of its history.
struct Unknown;

impl LastInputProbe for Unknown {
    fn read(&self) -> Option<LastInputReading> {
        None
    }
}

impl PowerBoundaryLog for Unknown {
    fn origin(&self) -> &'static str {
        "none"
    }

    fn read(&self, _: Timestamp, _: usize) -> Result<PowerBoundaries, HistoryUnavailable> {
        Err(HistoryUnavailable::NoSource)
    }
}

impl SessionBoundaryLog for Unknown {
    fn origin(&self) -> &'static str {
        "none"
    }

    fn lock_history(&self) -> LockHistory {
        LockHistory::NotRecorded
    }

    fn read(&self, _: Timestamp, _: usize) -> Result<SessionBoundaries, HistoryUnavailable> {
        Err(HistoryUnavailable::NoSource)
    }
}
