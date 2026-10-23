//! WorkTime's app services, ported from `WorkTime.App/Services` of the first .NET app: they read
//! the activity source and the ledger, run the domain rules over them and record what the user
//! decides. The crate stays platform independent; the Windows adapters live in
//! `worktime-windows`.

#![deny(unsafe_code)]

pub mod accounting;
pub mod corrections;
pub mod demo;
pub mod diagnostics;
pub mod history;
pub mod notifications;
pub mod ranges;
pub mod refresh;
pub mod reviews;
pub mod routing;
pub mod settings;
pub mod snapshot;
pub mod sources;
pub mod tracking;
pub mod usage_ping;
pub mod weekly;

#[cfg(test)]
mod test_support;
