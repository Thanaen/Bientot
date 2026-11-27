//! The notification server or the portal on the session bus, from a thread of its own: the app
//! hands it each notification and never waits on D-Bus, and it hears the clicks on the
//! connection that showed them, the one a server that addresses its signals sends them to.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::pin::pin;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use bientot_platform::notifications::{NotificationRequest, Notifier, QueuedClicks};
use futures_lite::StreamExt;
use zbus::message::Type;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{Connection, MatchRule, Message, MessageStream};

use super::{
    Delivered, Notify, PORTAL_BODY_ACTION, SERVER_BODY_ACTION, Service, notify, notify_once,
    portal_notification,
};
use crate::dbus::{BUS, BUS_PATH, call};

const SERVER: &str = "org.freedesktop.Notifications";
const SERVER_PATH: &str = "/org/freedesktop/Notifications";
const PORTAL: &str = "org.freedesktop.portal.Notification";
const PORTAL_DESKTOP: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

/// How long a call waits for its answer: the start waits on the first, which asks whether the
/// service answers at all.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Shows the reminders through `org.freedesktop.Notifications` or the portal, hands the clicks on
/// their body to the [`QueuedClicks`] it was started with, and tells the caller each show the
/// service refused. Dropping it ends its thread.
pub struct DesktopNotifications {
    shows: async_channel::Sender<NotificationRequest>,
}

/// No service answered: the app shows no reminder, so that none is reserved in the ledger
/// without being shown.
#[derive(Debug)]
pub struct Unavailable(String);

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for Unavailable {}

impl DesktopNotifications {
    /// Opens the session bus and waits, up to [`TIMEOUT`], for `service` to answer: the server's
    /// `GetServerInformation`, or the portal's `version`. A server that is not running may be
    /// started for it, as D-Bus activation does for notifications. `on_refused` runs on that
    /// thread for each show the service refuses later: the reminder stays reserved, as when a
    /// toast fails on Windows.
    pub fn start(
        service: Service,
        clicks: QueuedClicks,
        on_refused: impl FnMut() + Send + 'static,
    ) -> Result<Self, Unavailable> {
        let (shows, requests) = async_channel::unbounded();
        let (answer, answered) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("bientot-notifications".into())
            .spawn(move || {
                async_io::block_on(async move {
                    match open(service).await {
                        Ok(bus) => {
                            serve(service, bus, answer, requests, clicks, on_refused).await;
                        }
                        Err(error) => drop(answer.send(Err(error))),
                    }
                });
            })
            .map_err(|error| Unavailable(error.to_string()))?;
        answered.recv().unwrap_or_else(|_| {
            Err(Unavailable(
                "the notification thread ended before it opened".into(),
            ))
        })?;
        Ok(Self { shows })
    }
}

impl Notifier for DesktopNotifications {
    /// Returns once the notification thread has `request`. A refusal by the service comes later,
    /// through the `on_refused` of [`DesktopNotifications::start`].
    fn show(&self, request: &NotificationRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.shows
            .try_send(request.clone())
            .map_err(|_| "the notification thread ended".into())
    }
}

/// Shows `request` through `service` on a connection of its own, and returns once the service
/// took it or refused it, each call waiting up to [`TIMEOUT`]: for a process about to exit, which
/// hears no click. The server is sent [`notify_once`], which GNOME Shell keeps once the sender
/// has left the bus (gnome-shell `js/ui/notificationDaemon.js`); Plasma removes no notification
/// when its sender leaves (plasma-workspace `libnotificationmanager/server_p.cpp`). When the
/// application leaves, the portal only stops passing on the clicks on its notifications
/// (xdg-desktop-portal `src/notification.c`, `on_peer_disconnect`).
pub fn show_once(service: Service, request: &NotificationRequest) -> Result<(), Unavailable> {
    async_io::block_on(async {
        let bus = zbus::connection::Builder::session()?
            .method_timeout(TIMEOUT)
            .build()
            .await?;
        match service {
            Service::Server => show(&bus, &notify_once(request)).await.map(drop),
            Service::Portal => add(&bus, request).await,
        }
    })
    .map_err(|error| Unavailable(format!("{}: {error}", service.interface())))
}

/// Whether the service answered, which [`DesktopNotifications::start`] waits for.
type Answer = mpsc::SyncSender<Result<(), Unavailable>>;

enum Event<K> {
    Show(NotificationRequest),
    Invoked { key: K, action: String },
    Closed(K),
}

impl Service {
    fn interface(self) -> &'static str {
        match self {
            Self::Server => SERVER,
            Self::Portal => PORTAL,
        }
    }
}

async fn open(service: Service) -> Result<Connection, Unavailable> {
    connect(service)
        .await
        .map_err(|error| Unavailable(format!("{}: {error}", service.interface())))
}

/// Opens the session bus, asks `service` whether it answers, and listens to its signals before
/// anything is shown, so that no click goes unheard.
async fn connect(service: Service) -> zbus::Result<Connection> {
    let bus = zbus::connection::Builder::session()?
        .method_timeout(TIMEOUT)
        .build()
        .await?;
    let (destination, path, members): (_, _, &[&str]) = match service {
        Service::Server => {
            let _: (String, String, String, String) = call(
                &bus,
                SERVER,
                SERVER_PATH,
                SERVER,
                "GetServerInformation",
                &(),
            )
            .await?;
            (
                SERVER,
                SERVER_PATH,
                &["ActionInvoked", "NotificationClosed"],
            )
        }
        Service::Portal => {
            let _: OwnedValue = call(
                &bus,
                PORTAL_DESKTOP,
                PORTAL_PATH,
                "org.freedesktop.DBus.Properties",
                "Get",
                &(PORTAL, "version"),
            )
            .await?;
            (PORTAL_DESKTOP, PORTAL_PATH, &["ActionInvoked"])
        }
    };
    for member in members {
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(destination)?
            .interface(service.interface())?
            .path(path)?
            .member(*member)?
            .build()
            .to_string();
        call::<_, ()>(&bus, BUS, BUS_PATH, BUS, "AddMatch", &(rule,)).await?;
    }
    Ok(bus)
}

/// Shows each request and reports each click, until the app drops its [`DesktopNotifications`].
async fn serve(
    service: Service,
    bus: Connection,
    answer: Answer,
    requests: async_channel::Receiver<NotificationRequest>,
    clicks: QueuedClicks,
    mut on_refused: impl FnMut(),
) {
    let signals = MessageStream::from(&bus);
    let _ = answer.send(Ok(()));
    match service {
        Service::Server => {
            let signals = signals.filter_map(|message| server_signal(&message.ok()?));
            // The requests come last: the merged stream ends when they do.
            let mut events = pin!(signals.or(requests.map(Event::Show)));
            let mut delivered = Delivered::new(SERVER_BODY_ACTION);
            while let Some(event) = events.next().await {
                match event {
                    Event::Show(request) => match show(&bus, &notify(&request)).await {
                        Ok(id) => delivered.shown(id, request.delivery_id),
                        Err(_) => on_refused(),
                    },
                    Event::Invoked { key, action } => click(&clicks, &delivered, &key, &action),
                    Event::Closed(key) => delivered.closed(&key),
                }
            }
        }
        Service::Portal => {
            let signals = signals.filter_map(|message| portal_signal(&message.ok()?));
            let mut events = pin!(signals.or(requests.map(Event::Show)));
            let mut delivered = Delivered::new(PORTAL_BODY_ACTION);
            while let Some(event) = events.next().await {
                match event {
                    Event::Show(request) => match add(&bus, &request).await {
                        Ok(()) => delivered.shown(request.delivery_id.clone(), request.delivery_id),
                        Err(_) => on_refused(),
                    },
                    Event::Invoked { key, action } => click(&clicks, &delivered, &key, &action),
                    // The portal tells no close.
                    Event::Closed(key) => delivered.closed(&key),
                }
            }
        }
    }
}

async fn show(bus: &Connection, notify: &Notify<'_>) -> zbus::Result<u32> {
    let hints: HashMap<&str, Value<'_>> = notify
        .hints
        .iter()
        .map(|&(name, value)| (name, Value::from(value)))
        .collect();
    call(
        bus,
        SERVER,
        SERVER_PATH,
        SERVER,
        "Notify",
        &(
            notify.app_name,
            notify.replaces_id,
            notify.app_icon,
            notify.summary,
            notify.body,
            notify.actions.to_vec(),
            hints,
            notify.expire_timeout,
        ),
    )
    .await
}

/// The server's `ActionInvoked` or `NotificationClosed` a message carries.
fn server_signal(message: &Message) -> Option<Event<u32>> {
    let header = message.header();
    if header.message_type() != Type::Signal || header.interface()?.as_str() != SERVER {
        return None;
    }
    let body = message.body();
    match header.member()?.as_str() {
        "ActionInvoked" => {
            let (key, action): (u32, String) = body.deserialize().ok()?;
            Some(Event::Invoked { key, action })
        }
        "NotificationClosed" => {
            let (key, _reason): (u32, u32) = body.deserialize().ok()?;
            Some(Event::Closed(key))
        }
        _ => None,
    }
}

async fn add(bus: &Connection, request: &NotificationRequest) -> zbus::Result<()> {
    let shown = portal_notification(request);
    let notification = HashMap::from([
        ("title", Value::from(shown.title)),
        ("body", Value::from(shown.body)),
        ("default-action", Value::from(shown.default_action)),
    ]);
    call(
        bus,
        PORTAL_DESKTOP,
        PORTAL_PATH,
        PORTAL,
        "AddNotification",
        &(shown.id, notification),
    )
    .await
}

/// The portal's `ActionInvoked` a message carries.
fn portal_signal(message: &Message) -> Option<Event<String>> {
    let header = message.header();
    if header.message_type() != Type::Signal
        || header.interface()?.as_str() != PORTAL
        || header.member()?.as_str() != "ActionInvoked"
    {
        return None;
    }
    let (key, action, _parameter): (String, String, Vec<OwnedValue>) =
        message.body().deserialize().ok()?;
    Some(Event::Invoked { key, action })
}

fn click<K: Eq + std::hash::Hash>(
    clicks: &QueuedClicks,
    delivered: &Delivered<K>,
    key: &K,
    action: &str,
) {
    if let Some(delivery_id) = delivered.invoked(key, action) {
        clicks.click(Some(delivery_id.to_owned()));
    }
}

#[cfg(test)]
mod tests;
