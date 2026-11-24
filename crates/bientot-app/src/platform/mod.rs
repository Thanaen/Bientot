//! The composition root of the system integration. [`build`] makes, once per launch, the ports
//! the app runs on, from the backend of the system it was compiled for. The rest of the app takes
//! the ports and names no system: the platform conditions of the app belong in this module.
//!
//! What a launch needs before it holds its data folder is no part of [`Platform`]: the folder's
//! [`Instance`], the [`motion`] setting the startup failure window follows, and the functions
//! below.

#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(not(windows), path = "null.rs")]
mod backend;

use std::rc::Rc;
use std::sync::Arc;

use bientot_domain::sources::LastInputReading;
use bientot_platform::activity::LastInputProbe;
use bientot_platform::lifecycle::{InMemoryLifecycle, LifecycleSource};
use bientot_platform::notifications::{NotificationClicks, QueuedClicks};
use bientot_platform::placement::{Desktop, FlyoutPlacement, InMemoryDesktop};
use bientot_platform::power_history::{HistoryUnavailable, PowerBoundaries, PowerBoundaryLog};
use bientot_platform::session_history::{LockHistory, SessionBoundaries, SessionBoundaryLog};
use bientot_platform::startup::{LaunchIntent, StartupPort};
use bientot_platform::tray::{InMemoryTray, Tray, TrayEvent, TrayHost};
use bientot_store::data_directory::DataDirectory;
use futures::channel::mpsc;
use jiff::Timestamp;

use crate::launch::Launch;
use crate::live::LiveHost;
use crate::strings::Language;
use crate::tray_model::MenuCommand;

/// Holds the data folder: a later launch on it shows this one.
pub use backend::Instance;
pub use backend::build;
/// Installs the update a check downloaded, if any, and ends the process.
pub use backend::install_pending_update;
/// The folder that holds the user's data folder: `%LOCALAPPDATA%`, or the XDG data folder.
pub use backend::local_app_data;
/// Moves the open flyout's window so that its client area covers the bounds given, in physical
/// pixels: GPUI only resizes a window, and `bientot-platform` cannot name it.
pub use backend::place_window;
/// Runs the hooks of the installer before anything else; whether this launch may install a
/// pending update.
pub use backend::run_update_hooks;
/// Tells the user why Bientôt cannot start when no window can open.
pub use backend::show_fatal;
/// The notification-area icon's window, which owns the Store's prompts, once the icon shows.
#[cfg(windows)]
pub use backend::tray_window;
/// Waits up to a timeout for a process to exit; whether it did.
pub use backend::wait_for_exit;
/// Whether the user asked the system for less motion: [`motion`] reads it.
pub use backend::{Motion, motion};

/// What [`build`] knows of the launch.
pub struct PlatformContext<'a> {
    /// The data folder this launch holds.
    pub directory: &'a DataDirectory,
    pub launch: &'a Launch,
}

/// Has the live day show its reminders through the system's notifications, in its language, or
/// tells it that the system refused them. Nothing in a demo.
pub type EnableNotifications = Box<dyn FnOnce(&mut LiveHost<'static>, Language)>;

/// The clicks on the tray icon and the menu items chosen, in the order they came.
pub type TrayEvents = mpsc::UnboundedReceiver<TrayEvent>;

/// The ports of the system, for one launch.
pub struct Platform {
    /// How this launch started, by the user or at sign-in.
    pub launch_intent: LaunchIntent,
    pub last_input: Box<dyn LastInputProbe + Send>,
    pub lifecycle: Arc<dyn LifecycleSource + Send + Sync>,
    pub power_log: Box<dyn PowerBoundaryLog + Send>,
    pub session_log: Box<dyn SessionBoundaryLog + Send>,
    pub notifications: EnableNotifications,
    pub notification_clicks: Box<dyn NotificationClicks>,
    /// `None` in a demo, which leaves the user's registration alone, and where the system cannot
    /// start the app at sign-in.
    pub startup: Option<Arc<dyn StartupPort>>,
    /// Shows nothing until the app first publishes the day to it.
    pub tray: Rc<dyn Tray<MenuCommand>>,
    pub tray_events: TrayEvents,
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
            notifications: Box::new(|_, _| {}),
            notification_clicks: Box::new(QueuedClicks::default()),
            startup: None,
            tray: Rc::new(InMemoryTray::new(Vec::new(), None, TrayHost::Absent)),
            tray_events: mpsc::unbounded().1,
            desktop: Rc::new(InMemoryDesktop {
                placement: FlyoutPlacement::CompositorPlaced,
                usable_height: None,
                system_draws_frame: false,
            }),
        }
    }
}

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
