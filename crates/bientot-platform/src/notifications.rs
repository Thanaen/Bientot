//! What the app asks the system to show as a notification.

/// One toast: two lines of text, and the delivery id the receipts recorded it under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationRequest {
    pub delivery_id: String,
    pub title: String,
    pub body: String,
}
