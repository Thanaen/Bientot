//! WorkTime time accounting rules, ported from the `WorkTime.Domain` project of the first .NET app.
//!
//! The crate stays free of platform code so that it builds and tests on any OS.

#![forbid(unsafe_code)]

pub mod accounting;
pub mod adjustments;
pub mod configuration;
pub mod credit;
pub mod forecasting;
pub mod history;
pub mod intervals;
pub mod notifications;
pub mod sources;
pub mod summaries;
pub mod time;
pub mod usage_ping;
