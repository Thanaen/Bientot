//! WorkTime's Windows integration, ported from `WorkTime.Windows` of the first .NET app and the
//! services of `WorkTime.App` that drive it. The rules stay platform independent and tested on
//! any OS; only the Win32 calls behind them are Windows only.

// Only the Win32 calls in the `native` modules may opt out.
#![deny(unsafe_code)]

pub mod activity;
pub mod animation_effects;
pub mod diagnostics;
pub mod dialogs;
mod event_log;
pub mod lifecycle;
pub mod locale;
pub mod notifications;
pub mod power_history;
pub mod process;
#[cfg(windows)]
mod registry;
pub mod session_history;
pub mod single_instance;
pub mod startup;
pub mod updates;

/// True for the MSIX install. The portable build runs without package identity, so it has no
/// startup task and no Store to ask, and its toasts need an AppUserModelID of their own.
#[cfg(windows)]
pub(crate) fn has_package_identity() -> bool {
    windows::ApplicationModel::Package::Current()
        .and_then(|package| package.Id())
        .and_then(|id| id.FamilyName())
        .is_ok()
}
