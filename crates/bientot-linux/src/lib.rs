//! Bientôt's Linux integration (specs/005-linux-support). The rules stay platform independent and
//! tested on any OS; only the system calls behind them, in the `native` modules, are Linux only.

#![deny(unsafe_code)]

#[cfg(target_os = "linux")]
mod dbus;
pub mod idle;
pub mod logind;
pub mod notifications;
pub mod placement;
pub mod tray;

/// The desktop harness tests share one session bus and one logind mock, so they take turns.
#[cfg(all(test, target_os = "linux"))]
static HARNESS_TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
