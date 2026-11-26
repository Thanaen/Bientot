//! The logind session of this process on the system bus, and the screensaver on the session bus.
//!
//! SIGTERM is left to its default: the process ends, and every change already handed to the
//! listener is in the ledger, each in a transaction of its own. Catching it here would keep a
//! session from logging out; the app decides whether it quits on it.

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;

use bientot_domain::accounting::SessionState;
use bientot_domain::time::{Clock, TimeZoneProvider};
use bientot_platform::lifecycle::{
    LifecycleListener, LifecycleSource, LifecycleTransition, LockReporting, SessionSnapshot,
};
use futures_lite::StreamExt;
use futures_lite::stream::{self, Boxed};
use zbus::message::Type;
use zbus::zvariant::{DynamicType, ObjectPath, OwnedObjectPath, OwnedValue};
use zbus::{Connection, MatchRule, Message, MessageStream, match_rule};

use super::{SessionReadings, SessionTracker, Signal};
use crate::dbus::{BUS, BUS_PATH, call, has_owner};

const LOGIN1: &str = "org.freedesktop.login1";
const MANAGER_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";
const USER: &str = "org.freedesktop.login1.User";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// The screensaver interfaces, each with its object; each service is named after its interface.
const SCREENSAVERS: [(&str, &str); 2] = [
    (
        "org.freedesktop.ScreenSaver",
        "/org/freedesktop/ScreenSaver",
    ),
    ("org.gnome.ScreenSaver", "/org/gnome/ScreenSaver"),
];

/// The lock and the power of this process's logind session, from a system-bus thread of its own.
/// No inhibitor lock is taken: the ledger holds every change as it arrives.
pub struct LogindSource<C, Z> {
    clock: Arc<C>,
    zones: Arc<Z>,
    own: OwnSession,
}

impl<C, Z> LogindSource<C, Z>
where
    C: Clock + Send + Sync + 'static,
    Z: TimeZoneProvider + Send + Sync + 'static,
{
    /// Stamps each change with `clock` and the local zone of `zones` as it arrives.
    pub fn new(clock: C, zones: Z) -> Self {
        Self::for_session(clock, zones, OwnSession::of_this_process())
    }

    fn for_session(clock: C, zones: Z, own: OwnSession) -> Self {
        Self {
            clock: Arc::new(clock),
            zones: Arc::new(zones),
            own,
        }
    }
}

impl<C, Z> LifecycleSource for LogindSource<C, Z>
where
    C: Clock + Send + Sync + 'static,
    Z: TimeZoneProvider + Send + Sync + 'static,
{
    fn snapshot(&self) -> SessionSnapshot {
        async_io::block_on(async {
            let system = Connection::system().await.ok()?;
            let session = self.own.resolve(&system).await.ok()?;
            let session_bus = Connection::session().await.ok();
            let readings = read(&system, &session, session_bus.as_ref()).await.ok()?;
            Some(SessionTracker::new(readings).snapshot())
        })
        .unwrap_or(SessionSnapshot {
            session: SessionState::Unknown,
            lock_reporting: LockReporting::Unavailable,
        })
    }

    fn watch(&self, mut listener: Box<dyn LifecycleListener>) -> io::Result<Box<dyn Send>> {
        let (opened, opening) = mpsc::sync_channel(1);
        let (stop, stopped) = async_channel::bounded::<()>(1);
        let own = self.own.clone();
        let clock = Arc::clone(&self.clock);
        let zones = Arc::clone(&self.zones);
        let thread = std::thread::Builder::new()
            .name("bientot-logind".into())
            .spawn(move || {
                async_io::block_on(async move {
                    let (mut tracker, mut events) = match open(&own, stopped).await {
                        Ok(watching) => {
                            let _ = opened.send(Ok(()));
                            watching
                        }
                        Err(error) => {
                            let _ = opened.send(Err(error));
                            return;
                        }
                    };
                    while let Some(event) = events.next().await {
                        let change = match event {
                            Event::Signal(signal) => tracker.apply(signal),
                            Event::SystemBusClosed => return listener.ended(),
                            Event::Stop => return,
                        };
                        if let Some(change) = change {
                            listener.changed(LifecycleTransition {
                                change,
                                occurred_at: clock.now(),
                                time_zone_id: zones.local().0,
                            });
                        }
                    }
                })
            })?;
        let opened = opening.recv().unwrap_or_else(|_| {
            Err(io::Error::other(
                "the logind thread ended before watching the session",
            ))
        });
        let watch = LogindWatch {
            stop,
            thread: Some(thread),
        };
        opened.map(|()| Box::new(watch) as Box<dyn Send>)
    }
}

/// Stops the logind thread when dropped.
struct LogindWatch {
    stop: async_channel::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for LogindWatch {
    fn drop(&mut self) {
        self.stop.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum Event {
    Signal(Signal),
    /// The only end of the observation. `PrepareForShutdown(true)` is not one: logind sends
    /// `PrepareForShutdown(false)` after it when the shutdown fails.
    SystemBusClosed,
    Stop,
}

/// Subscribes to every signal, then reads the session, so that no change falls in between.
async fn open(
    own: &OwnSession,
    stopped: async_channel::Receiver<()>,
) -> io::Result<(SessionTracker, Boxed<Event>)> {
    let system = Connection::system().await.map_err(io::Error::other)?;
    let session = own.resolve(&system).await?;
    let session_bus = Connection::session().await.ok();
    let system_signals = [
        signal_rule(MANAGER, "PrepareForSleep")
            .and_then(|rule| rule.sender(LOGIN1)?.path(MANAGER_PATH)),
        signal_rule(PROPERTIES, "PropertiesChanged")
            .and_then(|rule| rule.sender(LOGIN1)?.path(&session)?.arg(0, SESSION)),
    ];
    let system_signals = subscribe(&system, system_signals)
        .await
        .map_err(io::Error::other)?
        .map(|message| message.map_or(Some(Event::SystemBusClosed), |message| decode(&message)))
        .chain(stream::once(Some(Event::SystemBusClosed)))
        .filter_map(|event| event);
    let screensaver = match &session_bus {
        Some(bus) => subscribe(
            bus,
            SCREENSAVERS.map(|(interface, _)| signal_rule(interface, "ActiveChanged")),
        )
        .await
        .ok(),
        None => None,
    };
    let screensaver = match screensaver {
        Some(signals) => signals
            .filter_map(|message| decode(&message.ok()?))
            .chain(stream::pending())
            .boxed(),
        None => stream::pending().boxed(),
    };
    let readings = read(&system, &session, session_bus.as_ref())
        .await
        .map_err(io::Error::other)?;
    let stop = stream::once_future(async move {
        let _ = stopped.recv().await;
        Event::Stop
    });
    let events = stop.or(system_signals.or(screensaver)).boxed();
    Ok((SessionTracker::new(readings), events))
}

fn signal_rule<'m>(interface: &'m str, member: &'m str) -> zbus::Result<match_rule::Builder<'m>> {
    MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(interface)?
        .member(member)
}

/// Every message of `bus` once it matches one of `rules`, in the order they arrive: one stream,
/// so that a lock and a suspend reach the tracker as logind sent them. It ends when the bus closes.
async fn subscribe<'m>(
    bus: &Connection,
    rules: impl IntoIterator<Item = zbus::Result<match_rule::Builder<'m>>>,
) -> zbus::Result<MessageStream> {
    let messages = MessageStream::from(bus);
    for rule in rules {
        let rule = rule?.build().to_string();
        call::<_, ()>(bus, BUS, BUS_PATH, BUS, "AddMatch", &(rule,)).await?;
    }
    Ok(messages)
}

/// The signal a message carries, or `None` for a message of another shape.
fn decode(message: &Message) -> Option<Event> {
    let header = message.header();
    let body = message.body();
    let signal = match (header.interface()?.as_str(), header.member()?.as_str()) {
        (MANAGER, "PrepareForSleep") => Signal::PrepareForSleep(body.deserialize().ok()?),
        (PROPERTIES, "PropertiesChanged") => {
            let (interface, changed, _): (String, HashMap<String, OwnedValue>, Vec<String>) =
                body.deserialize().ok()?;
            if interface != SESSION {
                return None;
            }
            let flag = |name| {
                changed
                    .get(name)
                    .and_then(|value| bool::try_from(value).ok())
            };
            Signal::SessionChanged {
                locked_hint: flag("LockedHint"),
                active: flag("Active"),
            }
        }
        (_, "ActiveChanged") => Signal::ScreenSaverActive(body.deserialize().ok()?),
        _ => return None,
    };
    Some(Event::Signal(signal))
}

async fn read(
    system: &Connection,
    session: &ObjectPath<'_>,
    session_bus: Option<&Connection>,
) -> zbus::Result<SessionReadings> {
    let screensaver = match session_bus {
        Some(bus) => screensaver_active(bus).await,
        None => None,
    };
    Ok(SessionReadings {
        locked_hint: property(system, session, SESSION, "LockedHint").await?,
        active: property(system, session, SESSION, "Active").await?,
        screensaver,
    })
}

/// `GetActive` of the first screensaver running. One that is not running is not started for it.
async fn screensaver_active(bus: &Connection) -> Option<bool> {
    for (interface, path) in SCREENSAVERS {
        if has_owner(bus, interface).await
            && let Ok(active) = call(bus, interface, path, interface, "GetActive", &()).await
        {
            return Some(active);
        }
    }
    None
}

/// How this process finds its logind session (contracts/linux-backends.md): the session of its
/// process id; else `XDG_SESSION_ID`; else the user's display session, as when the user manager
/// starts it outside any session.
#[derive(Clone, Debug)]
struct OwnSession {
    pid: u32,
    session_id: Option<String>,
}

impl OwnSession {
    fn of_this_process() -> Self {
        Self {
            pid: std::process::id(),
            session_id: std::env::var("XDG_SESSION_ID")
                .ok()
                .filter(|id| !id.is_empty()),
        }
    }

    async fn resolve(&self, system: &Connection) -> io::Result<OwnedObjectPath> {
        if let Ok(session) = manager(system, "GetSessionByPID", &(self.pid,)).await {
            return Ok(session);
        }
        if let Some(id) = &self.session_id
            && let Ok(session) = manager(system, "GetSession", &(id,)).await
        {
            return Ok(session);
        }
        match display_session(system).await {
            Ok(session) if session.as_str() != "/" => Ok(session),
            _ => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "logind knows no session of this process or its user",
            )),
        }
    }
}

/// The `Display` session of the user running this process: `/` when the user has none.
async fn display_session(system: &Connection) -> zbus::Result<OwnedObjectPath> {
    let own_name = system
        .unique_name()
        .ok_or_else(|| zbus::Error::Failure("the bus gave no name to this connection".into()))?;
    let uid: u32 = call(
        system,
        BUS,
        BUS_PATH,
        BUS,
        "GetConnectionUnixUser",
        &(own_name.as_str(),),
    )
    .await?;
    let user: OwnedObjectPath = manager(system, "GetUser", &(uid,)).await?;
    let (_, session): (String, OwnedObjectPath) = property(system, &user, USER, "Display").await?;
    Ok(session)
}

async fn manager<B>(system: &Connection, method: &str, body: &B) -> zbus::Result<OwnedObjectPath>
where
    B: zbus::export::serde::Serialize + DynamicType,
{
    call(system, LOGIN1, MANAGER_PATH, MANAGER, method, body).await
}

async fn property<T>(
    system: &Connection,
    object: &ObjectPath<'_>,
    interface: &str,
    name: &str,
) -> zbus::Result<T>
where
    T: TryFrom<OwnedValue>,
    T::Error: Into<zbus::Error>,
{
    let value: OwnedValue = call(
        system,
        LOGIN1,
        object.as_str(),
        PROPERTIES,
        "Get",
        &(interface, name),
    )
    .await?;
    T::try_from(value).map_err(Into::into)
}

#[cfg(test)]
mod tests;
