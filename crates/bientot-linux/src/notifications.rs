//! Reminders as desktop notifications (research.md, R9; contracts/linux-backends.md,
//! « Notifications »).
//!
//! Outside a sandbox the notification server, `org.freedesktop.Notifications`, shows them; inside
//! a Flatpak the notification portal does. [`notify`] and [`portal_notification`] write what each
//! is sent for a request; [`Delivered`] tells which of the signals they send back are clicks on
//! the body of a notification this process showed, and its delivery id. On Linux,
//! `DesktopNotifications` talks to them.

#[cfg(target_os = "linux")]
mod native;

use std::collections::HashMap;
use std::hash::Hash;

use bientot_platform::notifications::NotificationRequest;

use crate::APP_ID;

#[cfg(target_os = "linux")]
pub use native::{DesktopNotifications, Unavailable};

/// What shows the notifications.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    /// The notification server, `org.freedesktop.Notifications`.
    Server,
    /// `org.freedesktop.portal.Notification`, the one a sandbox reaches.
    Portal,
}

/// The action a server invokes for a click on the body of a notification.
pub const SERVER_BODY_ACTION: &str = "default";

/// The portal's default action. Its backends hand an action named `app.…` to the application's
/// own `org.freedesktop.Application` interface, which Bientôt does not serve, and send back
/// `ActionInvoked` only for other names.
pub const PORTAL_BODY_ACTION: &str = "open";

/// The arguments of the server's `Notify`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notify<'a> {
    pub app_name: &'static str,
    /// 0: a new notification, never one that replaces another.
    pub replaces_id: u32,
    pub app_icon: &'static str,
    pub summary: &'a str,
    pub body: &'a str,
    /// Identifier and label, in turn. The body's action has no label: a click on the body
    /// invokes it, and it shows no button.
    pub actions: [&'static str; 2],
    /// Hints whose value is a string.
    pub hints: [(&'static str, &'static str); 1],
    /// -1: the server's own delay.
    pub expire_timeout: i32,
}

/// What the server is sent to show `request`. Bientôt's texts hold none of `&`, `<` and `>`,
/// so a server that reads the body as markup shows it as written.
pub fn notify(request: &NotificationRequest) -> Notify<'_> {
    Notify {
        app_name: "Bientôt",
        replaces_id: 0,
        app_icon: APP_ID,
        summary: &request.title,
        body: &request.body,
        actions: [SERVER_BODY_ACTION, ""],
        hints: [("desktop-entry", APP_ID)],
        expire_timeout: -1,
    }
}

/// What the portal's `AddNotification` is sent to show `request`. The portal takes the icon
/// and the application's name from the sandbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortalNotification<'a> {
    /// The delivery id: the portal names a notification by the id the application gives it.
    pub id: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub default_action: &'static str,
}

pub fn portal_notification(request: &NotificationRequest) -> PortalNotification<'_> {
    PortalNotification {
        id: &request.delivery_id,
        title: &request.title,
        body: &request.body,
        default_action: PORTAL_BODY_ACTION,
    }
}

/// The delivery id of each notification this process showed, by the key the service names it
/// with: the server's id, or the portal's, which is the delivery id itself. The service tells
/// every client of the clicks on any application's notifications, so a click counts only on a
/// key this process was given.
#[derive(Debug)]
pub struct Delivered<K> {
    shown: HashMap<K, String>,
    body_action: &'static str,
}

impl<K: Eq + Hash> Delivered<K> {
    /// Notifications whose body invokes `body_action`.
    pub fn new(body_action: &'static str) -> Self {
        Self {
            shown: HashMap::new(),
            body_action,
        }
    }

    pub fn shown(&mut self, key: K, delivery_id: String) {
        self.shown.insert(key, delivery_id);
    }

    /// The delivery id of a click on the body of the notification `key`, which invoked
    /// `action`; `None` when it is no click on the body of one of this process's notifications.
    pub fn invoked(&self, key: &K, action: &str) -> Option<&str> {
        (action == self.body_action)
            .then(|| self.shown.get(key))
            .flatten()
            .map(String::as_str)
    }

    /// The notification `key` is gone: a later key may reuse its number.
    pub fn closed(&mut self, key: &K) {
        self.shown.remove(key);
    }
}

#[cfg(test)]
mod tests;
