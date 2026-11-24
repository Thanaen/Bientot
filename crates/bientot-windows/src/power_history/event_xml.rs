//! The power boundary an event of the System log describes.

use std::num::NonZeroU64;

use bientot_platform::power_history::{PowerBoundary, PowerBoundaryKind};

use crate::event_log::xml::{digits, event_data, instant, parse_document};

use super::{kind_of, source_key};

/// The boundary a record describes, or `None` when it is malformed or describes no boundary.
/// Suspend and resume happen at `TimeCreated`; boot and shutdown at the `StartTime` and
/// `StopTime` of their event data, which the kernel can log much later.
pub(super) fn parse_boundary(xml: &str) -> Option<PowerBoundary> {
    let event = parse_document(xml)?;
    let system = event.single("System")?;
    let provider = system.single("Provider")?.attribute("Name")?;
    let event_id = digits(&system.single("EventID")?.value())?;
    let record_id = NonZeroU64::new(digits(&system.single("EventRecordID")?.value())?)?;
    let created_at = instant(system.single("TimeCreated")?.attribute("SystemTime")?)?;
    let kind = kind_of(provider, u32::try_from(event_id).ok()?)?;
    let at = match kind {
        PowerBoundaryKind::Suspend | PowerBoundaryKind::Resume => created_at,
        PowerBoundaryKind::Boot => instant(&event_data(&event, "StartTime")?)?,
        PowerBoundaryKind::Shutdown => instant(&event_data(&event, "StopTime")?)?,
    };
    Some(PowerBoundary {
        kind,
        at,
        source_key: source_key(kind, record_id),
    })
}
