//! Against python-dbusmock's logind, which `scripts/linux-desktop-harness.sh` starts on a private
//! system bus. The mock and the session bus are shared by every test, so the tests take turns.

use std::sync::PoisonError;
use std::time::Duration;

use bientot_domain::time::FixedTimeZoneProvider;
use bientot_platform::PowerState;
use bientot_platform::lifecycle::LifecycleChange;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use zbus::zvariant::Value;

use super::*;

const MOCK: &str = "org.freedesktop.DBus.Mock";
const WAIT: Duration = Duration::from_secs(5);

struct At(Timestamp);

impl Clock for At {
    fn now(&self) -> Timestamp {
        self.0
    }
}

fn at() -> Timestamp {
    "2026-11-26T08:30:00Z".parse().unwrap()
}

fn source(pid: u32, session_id: Option<&str>) -> LogindSource<At, FixedTimeZoneProvider> {
    source_of(InstallKind::AppImage, pid, session_id)
}

/// The source of an install of `kind` whose process has `pid` and `XDG_SESSION_ID`.
fn source_of(
    kind: InstallKind,
    pid: u32,
    session_id: Option<&str>,
) -> LogindSource<At, FixedTimeZoneProvider> {
    LogindSource::for_session(
        At(at()),
        FixedTimeZoneProvider::new("Europe/Paris", TimeZone::UTC),
        OwnSession::new(
            SessionLookup::for_install(kind),
            pid,
            session_id.map(Into::into),
        ),
    )
}

/// Drives the logind mock.
struct Logind(Connection);

impl Logind {
    /// Gives the mock a `GetSessionByPID` that knows the session `p<pid>` of each pid, and a
    /// `GetUser` that works: the template's adds a number to a string.
    fn connect() -> Self {
        let logind = Self(async_io::block_on(Connection::system()).unwrap());
        logind.mock(
            MANAGER_PATH,
            "AddMethod",
            &(
                MANAGER,
                "GetUser",
                "u",
                "o",
                "ret = '/org/freedesktop/login1/user/%d' % args[0]",
            ),
        );
        logind.mock(
            MANAGER_PATH,
            "AddMethod",
            &(
                MANAGER,
                "GetSessionByPID",
                "u",
                "o",
                "ret = '/org/freedesktop/login1/session/p%d' % args[0]\n\
                 if ret not in objects:\n    \
                     raise dbus.exceptions.DBusException('no session', \
                     name='org.freedesktop.login1.NoSessionForPID')",
            ),
        );
        logind
    }

    fn mock<B: zbus::export::serde::Serialize + DynamicType>(
        &self,
        path: &str,
        method: &str,
        body: &B,
    ) {
        async_io::block_on(
            self.0
                .call_method(Some(LOGIN1), path, Some(MOCK), method, body),
        )
        .unwrap_or_else(|error| panic!("the logind mock refused {method}: {error}"));
    }

    /// Adds the session `id` of this user, unlocked.
    fn add_session(&self, id: &str, active: bool) -> String {
        let uid = async_io::block_on(own_uid(&self.0));
        self.mock(
            MANAGER_PATH,
            "AddSession",
            &(id, "seat0", uid, "harness", active),
        );
        format!("/org/freedesktop/login1/session/{id}")
    }

    fn set(&self, session: &str, properties: &[(&str, bool)]) {
        let properties: HashMap<&str, Value<'_>> = properties
            .iter()
            .map(|&(name, value)| (name, Value::from(value)))
            .collect();
        self.mock(session, "UpdateProperties", &(SESSION, properties));
    }

    fn emit(&self, member: &str, value: bool) {
        self.mock(
            MANAGER_PATH,
            "EmitSignal",
            &(MANAGER, member, "b", vec![Value::from(value)]),
        );
    }

    fn set_display(&self, session: &str) {
        let uid = async_io::block_on(own_uid(&self.0));
        let user = format!("/org/freedesktop/login1/user/{uid}");
        let id = session.rsplit('/').next().unwrap_or_default();
        let display = Value::new((id, ObjectPath::try_from(session).unwrap()));
        async_io::block_on(self.0.call_method(
            Some(LOGIN1),
            user.as_str(),
            Some(PROPERTIES),
            "Set",
            &(USER, "Display", display),
        ))
        .unwrap();
    }
}

async fn own_uid(bus: &Connection) -> u32 {
    let own_name = bus.unique_name().unwrap().as_str();
    call(
        bus,
        BUS,
        BUS_PATH,
        BUS,
        "GetConnectionUnixUser",
        &(own_name,),
    )
    .await
    .unwrap()
}

#[derive(Debug, PartialEq, Eq)]
enum Heard {
    Changed(LifecycleChange),
    Ended,
}

struct Ears(mpsc::Sender<Delivery>);

/// A change, or `None` once the watch ended.
type Delivery = Option<LifecycleTransition>;

impl LifecycleListener for Ears {
    fn changed(&mut self, transition: LifecycleTransition) {
        let _ = self.0.send(Some(transition));
    }

    fn ended(&mut self) {
        let _ = self.0.send(None);
    }
}

/// The first `count` things the watch hands its listener.
fn heard(received: &mpsc::Receiver<Delivery>, count: usize) -> Vec<Heard> {
    let mut heard = Vec::new();
    while heard.len() < count {
        match received.recv_timeout(WAIT) {
            Ok(Some(transition)) => {
                assert_eq!(
                    (transition.occurred_at, transition.time_zone_id.as_str()),
                    (at(), "Europe/Paris")
                );
                heard.push(Heard::Changed(transition.change));
            }
            Ok(None) => heard.push(Heard::Ended),
            Err(error) => panic!("the watch went quiet after {heard:?}: {error}"),
        }
    }
    heard
}

fn watch(
    source: &LogindSource<At, FixedTimeZoneProvider>,
) -> (Box<dyn Send>, mpsc::Receiver<Delivery>) {
    let (sender, received) = mpsc::channel();
    let watch = source.watch(Box::new(Ears(sender))).unwrap();
    (watch, received)
}

fn live(session: SessionState) -> SessionSnapshot {
    SessionSnapshot {
        session,
        lock_reporting: LockReporting::Live,
    }
}

fn unknown_snapshot() -> SessionSnapshot {
    SessionSnapshot {
        session: SessionState::Unknown,
        lock_reporting: LockReporting::Unavailable,
    }
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_finds_the_session_of_the_process_then_of_xdg_session_id_then_the_display() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let logind = Logind::connect();
    let locked = |id| {
        let session = logind.add_session(id, true);
        logind.set(&session, &[("LockedHint", true)]);
        session
    };
    locked("p4101");
    logind.add_session("x4101", true);
    locked("x4102");
    let display = locked("d4103");
    logind.add_session("other4103", true);

    let by_pid = source(4101, Some("x4101")).snapshot();
    let by_session_id = source(4102, Some("x4102")).snapshot();
    logind.set_display(&display);
    let by_display = source(4103, None).snapshot();
    logind.set_display("/");
    let none = source(4104, None);

    assert_eq!(
        [by_pid, by_session_id, by_display, none.snapshot()],
        [
            live(SessionState::Locked),
            live(SessionState::Locked),
            live(SessionState::Locked),
            unknown_snapshot()
        ]
    );
    let refused = none.watch(Box::new(Ears(mpsc::channel().0))).err();
    assert_eq!(
        refused.map(|error| error.kind()),
        Some(io::ErrorKind::NotFound)
    );
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_a_flatpak_skips_the_session_of_its_process_id() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let logind = Logind::connect();
    // In the sandbox the process id names another process on the host, here one of a session
    // that never set `LockedHint`, whose lock state is unknown.
    logind.add_session("p4105", true);
    let own = logind.add_session("x4105", true);
    logind.set(&own, &[("LockedHint", true)]);

    let snapshots = [InstallKind::AppImage, InstallKind::Flatpak]
        .map(|kind| source_of(kind, 4105, Some("x4105")).snapshot());

    assert_eq!(snapshots, [unknown_snapshot(), live(SessionState::Locked)]);
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_reports_lock_switch_sleep_and_resume_of_the_session_through_a_failed_shutdown() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let logind = Logind::connect();
    let session = logind.add_session("w5101", true);
    let other = logind.add_session("o5101", true);
    let source = source(5101, Some("w5101"));
    let (_watch, received) = watch(&source);

    logind.set(&session, &[("LockedHint", true)]);
    logind.set(&session, &[("LockedHint", true)]);
    logind.set(&other, &[("LockedHint", false), ("Active", false)]);
    logind.set(&session, &[("LockedHint", false)]);
    logind.set(&session, &[("Active", false)]);
    logind.set(&session, &[("Active", true)]);
    logind.set(&session, &[("LockedHint", true), ("Active", false)]);
    logind.set(&session, &[("LockedHint", false), ("Active", true)]);
    logind.emit("PrepareForSleep", true);
    logind.emit("PrepareForSleep", false);
    logind.emit("PrepareForShutdown", true);
    logind.emit("PrepareForShutdown", false);
    logind.set(&session, &[("LockedHint", true)]);

    use Heard::Changed;
    use LifecycleChange::{Power, Session};
    let expected = [
        Changed(Session(SessionState::Locked)),
        Changed(Session(SessionState::Unlocked)),
        Changed(Session(SessionState::Locked)),
        Changed(Session(SessionState::Unlocked)),
        Changed(Session(SessionState::Locked)),
        Changed(Session(SessionState::Unlocked)),
        Changed(Power(PowerState::Suspended)),
        Changed(Power(PowerState::Active)),
        Changed(Session(SessionState::Locked)),
    ];
    assert_eq!(heard(&received, expected.len()), expected);
}

/// The screensaver of a desktop that never sets `LockedHint`, on the session bus.
struct ScreenSaver;

#[zbus::interface(name = "org.freedesktop.ScreenSaver")]
impl ScreenSaver {
    fn get_active(&self) -> bool {
        false
    }
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_takes_the_lock_from_the_screensaver_until_locked_hint_is_seen() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let logind = Logind::connect();
    let session = logind.add_session("s6101", true);
    let source = source(6101, Some("s6101"));
    assert_eq!(source.snapshot(), unknown_snapshot());
    let (name, path) = SCREENSAVERS[0];
    let screensaver = async_io::block_on(async {
        zbus::connection::Builder::session()?
            .name(name)?
            .serve_at(path, ScreenSaver)?
            .build()
            .await
    })
    .unwrap();
    let screensaver_active = |active: bool| {
        async_io::block_on(screensaver.emit_signal(
            None::<&str>,
            path,
            name,
            "ActiveChanged",
            &(active,),
        ))
        .unwrap();
        // The session bus is another socket: give the watch the time to read it before the next
        // change on the system bus.
        std::thread::sleep(Duration::from_millis(200));
    };
    let (_watch, received) = watch(&source);

    screensaver_active(true);
    screensaver_active(false);
    logind.set(&session, &[("LockedHint", true)]);
    screensaver_active(false);
    logind.set(&session, &[("LockedHint", false)]);
    screensaver_active(true);

    use Heard::Changed;
    use LifecycleChange::Session;
    let expected = [
        Changed(Session(SessionState::Locked)),
        Changed(Session(SessionState::Unlocked)),
        Changed(Session(SessionState::Locked)),
        Changed(Session(SessionState::Unlocked)),
    ];
    assert_eq!(heard(&received, expected.len()), expected);
    assert_eq!(source.snapshot(), live(SessionState::Unlocked));
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_dropping_the_watch_stops_its_thread() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let logind = Logind::connect();
    logind.add_session("q8101", true);
    let (watch, received) = watch(&source(8101, Some("q8101")));

    drop(watch);

    assert_eq!(received.try_recv(), Err(mpsc::TryRecvError::Disconnected));
}
