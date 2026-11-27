//! Bientôt's Linux integration (specs/005-linux-support). The rules stay platform independent and
//! tested on any OS; only the system calls behind them, in the `native` modules, are Linux only.

#![deny(unsafe_code)]

pub mod autostart;
#[cfg(target_os = "linux")]
mod dbus;
pub mod idle;
pub mod locale;
pub mod logind;
pub mod notifications;
pub mod placement;
pub mod process;
pub mod single_instance;
pub mod system;
pub mod tray;

/// The application id the desktop files and the Flatpak use, which names the icon too.
pub const APP_ID: &str = "dev.bientot.Bientot";

/// Whether `path` is absolute as Linux reads it: it starts at the root. `Path::is_absolute` also
/// wants a drive on Windows, where these rules are tested too.
fn is_absolute(path: &std::path::Path) -> bool {
    path.has_root()
}

/// The desktop harness tests share one session bus and one logind mock, so they take turns.
#[cfg(all(test, target_os = "linux"))]
static HARNESS_TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
