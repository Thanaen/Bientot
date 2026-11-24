//! The platform-independent half of Bientôt's system integration: the built-in activity source,
//! the recording of session and power changes, the recovery of power and session history, the
//! update check, and the types the platform crates share. `bientot-windows` makes the Windows
//! calls behind them.

#![forbid(unsafe_code)]

pub mod activity;
pub mod lifecycle;
pub mod notifications;
pub mod power_history;
pub mod session_history;
pub mod startup;
pub mod updates;

use jiff::Timestamp;

/// Whether the computer is running or about to sleep, as Windows reports it in `WM_POWERBROADCAST`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerState {
    Active,
    Suspended,
}

/// `DateTime.Ticks` of the instant in UTC: 100 ns intervals since 0001-01-01, as the .NET
/// operation keys spell instants.
pub fn dotnet_ticks(instant: Timestamp) -> i128 {
    const UNIX_EPOCH_TICKS: i128 = 621_355_968_000_000_000;
    instant.as_nanosecond().div_euclid(100) + UNIX_EPOCH_TICKS
}
