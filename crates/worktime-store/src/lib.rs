//! WorkTime persistence, ported from `WorkTime.Persistence` of the first .NET app: the data
//! directory, its `settings.json`, the SQLite ledger and the secrets. The Rust app reads the data
//! the .NET app wrote, so the paths, the files, the schema and every stored text keep the .NET
//! encodings.

// Only the DPAPI calls in `secrets::dpapi` may opt out.
#![deny(unsafe_code)]

pub mod activity;
pub mod adjustments;
pub mod app_state;
pub mod codec;
pub mod credit;
pub mod data_directory;
pub mod data_migration;
pub mod database;
mod file;
pub mod history;
pub mod notifications;
pub mod reviews;
mod row;
pub mod secrets;
pub mod sessions;
pub mod settings;
pub mod transfer;

pub use row::{InvalidStoredValue, StorageError};

#[cfg(test)]
mod scenario_tests;
#[cfg(test)]
mod test_support;
