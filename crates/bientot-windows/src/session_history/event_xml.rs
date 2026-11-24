//! The session boundary an event of the Winlogon log describes.

use std::num::NonZeroU64;

use bientot_platform::session_history::{SessionBoundary, SessionBoundaryKind};

use crate::event_log::xml::{digits, event_data, instant, parse_document};

const PROVIDER: &str = "Microsoft-Windows-Winlogon";

/// Logged when a subscriber begins handling a notification.
pub(super) const NOTIFICATION_STARTED: u64 = 811;

/// A boundary and the security identifier of the user whose session it changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Notification {
    pub user_id: String,
    pub boundary: SessionBoundary,
}

/// The kind of a Winlogon notification, by the number its `Event` data gives it. A session
/// disconnected by Switch User reads as locked, and reconnected as unlocked. Screen saver and
/// system notifications are not boundaries.
fn kind_of(notification: u64) -> Option<SessionBoundaryKind> {
    match notification {
        2 | 12 => Some(SessionBoundaryKind::Logon),
        3 | 13 => Some(SessionBoundaryKind::Logoff),
        4 | 8 => Some(SessionBoundaryKind::Lock),
        5 | 9 => Some(SessionBoundaryKind::Unlock),
        _ => None,
    }
}

/// `None` when the record is malformed; `Some(None)` when it is well formed but describes no
/// boundary: another event, or a notification that does not change the session.
pub(super) fn parse_notification(xml: &str) -> Option<Option<Notification>> {
    let event = parse_document(xml)?;
    let system = event.single("System")?;
    let provider = system.single("Provider")?.attribute("Name")?;
    let event_id = digits(&system.single("EventID")?.value())?;
    let record_id = NonZeroU64::new(digits(&system.single("EventRecordID")?.value())?)?;
    let at = instant(system.single("TimeCreated")?.attribute("SystemTime")?)?;
    if provider != PROVIDER || event_id != NOTIFICATION_STARTED {
        return Some(None);
    }
    let notification = digits(event_data(&event, "Event")?.trim())?;
    let Some(kind) = kind_of(notification) else {
        return Some(None);
    };
    let user_id = system.single("Security")?.attribute("UserID")?.to_owned();
    Some(Some(Notification {
        user_id,
        boundary: SessionBoundary {
            kind,
            at,
            // The log reuses record IDs after it is cleared; the recorded key adds the instant.
            source_key: record_id.to_string(),
        },
    }))
}
