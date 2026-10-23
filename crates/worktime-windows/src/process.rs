//! Waiting for the WorkTime that restarts this one to end, so that this one can claim the data
//! folder and open its ledger.

#[cfg(windows)]
mod native;

#[cfg(windows)]
pub use native::wait_for_exit;
