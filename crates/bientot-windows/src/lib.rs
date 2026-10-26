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
pub mod notifications;
pub mod power_history;
pub mod process;
#[cfg(windows)]
mod registry;
pub mod session_history;
pub mod single_instance;
pub mod startup;
pub mod updates;

use jiff::Timestamp;

/// Whether the computer is running or about to sleep, as Windows reports it in `WM_POWERBROADCAST`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerState {
    Active,
    Suspended,
}

/// True for the MSIX install. The portable build runs without package identity, so it has no
/// startup task and no Store to ask, and its toasts need an AppUserModelID of their own.
#[cfg(windows)]
pub(crate) fn has_package_identity() -> bool {
    windows::ApplicationModel::Package::Current()
        .and_then(|package| package.Id())
        .and_then(|id| id.FamilyName())
        .is_ok()
}

/// `DateTime.Ticks` of the instant in UTC: 100 ns intervals since 0001-01-01, as the .NET
/// operation keys spell instants.
fn dotnet_ticks(instant: Timestamp) -> i128 {
    const UNIX_EPOCH_TICKS: i128 = 621_355_968_000_000_000;
    instant.as_nanosecond().div_euclid(100) + UNIX_EPOCH_TICKS
}
