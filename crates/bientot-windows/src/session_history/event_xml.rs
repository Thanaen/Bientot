//! The session history boundary an event of the Winlogon log describes.

use std::num::NonZeroU64;

use crate::event_log::xml::{digits, event_data, instant, parse_document};

use super::{SessionHistoryBoundary, SessionHistoryBoundaryKind};

const PROVIDER: &str = "Microsoft-Windows-Winlogon";

/// Logged when a subscriber begins handling a notification.
pub(super) const NOTIFICATION_STARTED: u64 = 811;

/// A boundary and the security identifier of the user whose session it changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Notification {
    pub user_id: String,
    pub boundary: SessionHistoryBoundary,
}

/// `None` when the record is malformed; `Some(None)` when it is well formed but describes no
/// boundary: another event, or a notification that does not change the session.
pub(super) fn parse_notification(xml: &str) -> Option<Option<Notification>> {
    let event = parse_document(xml)?;
    let system = event.single("System")?;
    let provider = system.single("Provider")?.attribute("Name")?;
    let event_id = digits(&system.single("EventID")?.value())?;
    let record_id = NonZeroU64::new(digits(&system.single("EventRecordID")?.value())?)?;
    let occurred_at = instant(system.single("TimeCreated")?.attribute("SystemTime")?)?;
    if provider != PROVIDER || event_id != NOTIFICATION_STARTED {
        return Some(None);
    }
    let notification = digits(event_data(&event, "Event")?.trim())?;
    let Some(kind) = SessionHistoryBoundaryKind::from_notification(notification) else {
        return Some(None);
    };
    let user_id = system.single("Security")?.attribute("UserID")?.to_owned();
    Some(Some(Notification {
        user_id,
        boundary: SessionHistoryBoundary {
            kind,
            record_id,
            occurred_at,
        },
    }))
}
