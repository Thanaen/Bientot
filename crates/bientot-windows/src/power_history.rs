//! The power boundaries of the System event log, ported from `Power/WindowsPowerHistoryPort.cs`.
//! See `docs/power-history-recovery.md`.
//!
//! Only the query of the log is Windows only; the event XML is parsed here, and
//! `bientot_platform::power_history` pairs and records what it describes.

#![cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "only the Windows System log decodes records outside the tests"
    )
)]

mod event_xml;
#[cfg(windows)]
mod native;

use std::num::NonZeroU64;

use bientot_platform::power_history::{
    HistoryUnavailable, PowerBoundaries, PowerBoundaryKind, PowerBoundaryLog,
};
use jiff::Timestamp;

use crate::event_log::CollectedRecords;

#[cfg(windows)]
pub use native::system_log;

const KERNEL_GENERAL: &str = "Microsoft-Windows-Kernel-General";
const KERNEL_POWER: &str = "Microsoft-Windows-Kernel-Power";

/// The provider and event ID that log each boundary.
const EVENTS: [(PowerBoundaryKind, &str, u32); 4] = [
    (PowerBoundaryKind::Shutdown, KERNEL_GENERAL, 13),
    (PowerBoundaryKind::Boot, KERNEL_GENERAL, 12),
    (PowerBoundaryKind::Suspend, KERNEL_POWER, 42),
    (PowerBoundaryKind::Resume, KERNEL_POWER, 107),
];

fn kind_of(provider: &str, event_id: u32) -> Option<PowerBoundaryKind> {
    EVENTS
        .into_iter()
        .find(|&(_, logged_by, id)| (logged_by, id) == (provider, event_id))
        .map(|(kind, ..)| kind)
}

/// The provider, event ID and `EventRecordID` of the record, which the log reuses after it is
/// cleared, so it identifies a boundary only together with the instant.
fn source_key(kind: PowerBoundaryKind, record_id: NonZeroU64) -> String {
    let (_, provider, event_id) = EVENTS
        .into_iter()
        .find(|&(logged, ..)| logged == kind)
        .expect("every kind is logged");
    format!("{}:{event_id}:{record_id}", provider.to_lowercase())
}

/// The System log could not be queried or read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SystemLogUnavailable;

/// The kernel's boot, shutdown, suspend and resume records in the System log.
pub(crate) trait SystemEventLog {
    fn read(&self, record_limit: usize) -> Result<CollectedRecords, SystemLogUnavailable>;
}

/// The power boundaries the records of a [`SystemEventLog`] describe.
pub(crate) struct SystemLogBoundaries<L>(pub(crate) L);

impl<L: SystemEventLog> PowerBoundaryLog for SystemLogBoundaries<L> {
    fn origin(&self) -> &'static str {
        "windows-power-history"
    }

    /// The whole log, whatever `since`: a boot or shutdown happens at an instant of its event
    /// data, which the kernel can log much later than it.
    fn read(
        &self,
        _since: Timestamp,
        record_limit: usize,
    ) -> Result<PowerBoundaries, HistoryUnavailable> {
        let records = match self.0.read(record_limit) {
            Ok(CollectedRecords::Complete(records)) => records,
            Ok(CollectedRecords::Truncated) => {
                return Ok(PowerBoundaries {
                    boundaries: Vec::new(),
                    complete: false,
                });
            }
            Err(SystemLogUnavailable) => {
                return Err(HistoryUnavailable::Failed(
                    "the System log could not be queried or read".to_owned(),
                ));
            }
        };
        records
            .iter()
            .map(|xml| event_xml::parse_boundary(xml))
            .collect::<Option<Vec<_>>>()
            .map(|boundaries| PowerBoundaries {
                boundaries,
                complete: true,
            })
            .ok_or(HistoryUnavailable::Malformed)
    }
}

#[cfg(test)]
mod tests;
