//! The power history boundary an event of the System log describes.

use std::num::NonZeroU64;

use crate::event_log::xml::{digits, event_data, instant, parse_document};

use super::{PowerHistoryBoundary, PowerHistoryBoundaryKind};

/// The boundary a record describes, or `None` when it is malformed or describes no boundary.
/// Suspend and resume happen at `TimeCreated`; boot and shutdown at the `StartTime` and
/// `StopTime` of their event data, which the kernel can log much later.
pub(super) fn parse_boundary(xml: &str) -> Option<PowerHistoryBoundary> {
    let event = parse_document(xml)?;
    let system = event.single("System")?;
    let provider = system.single("Provider")?.attribute("Name")?;
    let event_id = digits(&system.single("EventID")?.value())?;
    let record_id = NonZeroU64::new(digits(&system.single("EventRecordID")?.value())?)?;
    let created_at = instant(system.single("TimeCreated")?.attribute("SystemTime")?)?;
    let kind = PowerHistoryBoundaryKind::from_event(provider, u32::try_from(event_id).ok()?)?;
    let occurred_at = match kind {
        PowerHistoryBoundaryKind::Suspend | PowerHistoryBoundaryKind::Resume => created_at,
        PowerHistoryBoundaryKind::Boot => instant(&event_data(&event, "StartTime")?)?,
        PowerHistoryBoundaryKind::Shutdown => instant(&event_data(&event, "StopTime")?)?,
    };
    Some(PowerHistoryBoundary {
        kind,
        record_id,
        occurred_at,
    })
}
