//! Against python-dbusmock's notification server, which `scripts/linux-desktop-harness.sh` starts
//! on the session bus, and a notification portal each test starts itself. The tests share the
//! bus, so they take turns. Each plays the desktop: it reads what the service was asked to show
//! from the mock's call log, and clicks by having the mock send the service's signal.

use std::process::{Child, Command, Stdio};
use std::sync::PoisonError;
use std::sync::mpsc::{self, Receiver};
use std::thread::sleep;
use std::time::{Duration, Instant};

use bientot_platform::notifications::NotificationClicks as _;
use zbus::zvariant::{Array, OwnedValue, Value};

use super::*;

const MOCK: &str = "org.freedesktop.DBus.Mock";
const WAIT: Duration = Duration::from_secs(5);
/// How long a click that should not come is waited for.
const QUIET: Duration = Duration::from_millis(500);

fn session() -> Connection {
    async_io::block_on(Connection::session()).unwrap()
}

fn request(delivery_id: &str) -> NotificationRequest {
    NotificationRequest {
        delivery_id: delivery_id.into(),
        title: "Bientôt".into(),
        body: format!(
            "Il reste 15 minutes avant votre objectif. Départ prévu : 17:55. ({delivery_id})"
        ),
    }
}

/// Started on `service`, with the delivery ids of the clicks it reports.
fn started(
    service: Service,
) -> Result<(DesktopNotifications, Receiver<Option<String>>), Unavailable> {
    let clicks = QueuedClicks::default();
    let (sender, received) = mpsc::channel();
    clicks.on_click(Box::new(move |delivery_id| {
        sender.send(delivery_id.map(str::to_owned)).ok();
    }));
    DesktopNotifications::start(service, clicks).map(|notifications| (notifications, received))
}

/// Waits up to [`WAIT`] for `check` to give a value.
fn eventually<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "gave up waiting: {what}");
        sleep(Duration::from_millis(50));
    }
}

fn mock_call<B, R>(bus: &Connection, destination: &str, path: &str, method: &str, body: &B) -> R
where
    B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> zbus::export::serde::Deserialize<'d> + zbus::zvariant::Type,
{
    async_io::block_on(call(bus, destination, path, MOCK, method, body))
        .unwrap_or_else(|error| panic!("{method} on the mock of {destination}: {error}"))
}

/// The arguments of each call of `method` the mock answered, oldest first.
fn calls(bus: &Connection, destination: &str, path: &str, method: &str) -> Vec<Vec<OwnedValue>> {
    let calls: Vec<(u64, Vec<OwnedValue>)> =
        mock_call(bus, destination, path, "GetMethodCalls", &(method,));
    calls.into_iter().map(|(_, arguments)| arguments).collect()
}

/// Has the mock of `destination` send the signal `name` of `interface`.
fn emit(
    bus: &Connection,
    destination: &str,
    path: &str,
    interface: &str,
    name: &str,
    signature: &str,
    arguments: Vec<Value<'_>>,
) {
    mock_call::<_, ()>(
        bus,
        destination,
        path,
        "EmitSignal",
        &(interface, name, signature, arguments),
    );
}

fn text(value: &OwnedValue) -> String {
    String::try_from(value.try_clone().unwrap()).unwrap()
}

/// The arguments of the server's `Notify` call whose body is `body`.
fn notified(bus: &Connection, body: &str) -> Option<Vec<OwnedValue>> {
    calls(bus, SERVER, SERVER_PATH, "Notify")
        .into_iter()
        .find(|arguments| text(&arguments[4]) == body)
}

/// Has the server mock number the next new notifications from `id`, as it numbers them in turn.
fn number_from(bus: &Connection, id: u32) {
    mock_call::<_, ()>(
        bus,
        SERVER,
        SERVER_PATH,
        "AddMethod",
        &(SERVER, "NumberFrom", "u", "", "self.next_id = args[0]"),
    );
    async_io::block_on(call::<_, ()>(
        bus,
        SERVER,
        SERVER_PATH,
        SERVER,
        "NumberFrom",
        &(id,),
    ))
    .unwrap();
}

fn server_clicks(bus: &Connection, id: u32, action: &str) {
    emit(
        bus,
        SERVER,
        SERVER_PATH,
        SERVER,
        "ActionInvoked",
        "us",
        vec![id.into(), action.into()],
    );
}

/// A method of the server mock answering an error for the life of the value, as a server that
/// refuses it.
struct Refusing<'b> {
    bus: &'b Connection,
    method: &'static str,
    signature: (&'static str, &'static str),
    /// What python-dbusmock's template answers, put back when dropped.
    answer: &'static str,
}

impl<'b> Refusing<'b> {
    fn new(bus: &'b Connection, method: &'static str) -> Self {
        let (signature, answer) = match method {
            "GetServerInformation" => (
                ("", "ssss"),
                r#"ret = ("mock-notify", "test vendor", "1.0", "1.1")"#,
            ),
            "Notify" => (
                ("susssasa{sv}i", "u"),
                "if args[1]:\n    ret = args[1]\nelse:\n    ret = self.next_id\n    self.next_id += 1\n",
            ),
            _ => unreachable!("the test refuses {method}"),
        };
        let refusing = Self {
            bus,
            method,
            signature,
            answer,
        };
        refusing.answer_with(
            "raise dbus.exceptions.DBusException('refused', name='org.freedesktop.DBus.Error.Failed')",
        );
        refusing
    }

    fn answer_with(&self, code: &str) {
        let (input, output) = self.signature;
        mock_call::<_, ()>(
            self.bus,
            SERVER,
            SERVER_PATH,
            "AddMethod",
            &(SERVER, self.method, input, output, code),
        );
    }
}

impl Drop for Refusing<'_> {
    fn drop(&mut self) {
        self.answer_with(self.answer);
    }
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_shows_the_text_and_hints_and_reports_the_clicks_on_the_body() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let bus = session();
    let (notifications, clicks) = started(Service::Server).unwrap();
    number_from(&bus, 7001);

    for delivery_id in ["text:first", "text:second"] {
        notifications.show(&request(delivery_id)).unwrap();
    }
    let first = request("text:first");
    let arguments = eventually("the first notification", || notified(&bus, &first.body));
    eventually("the second notification", || {
        notified(&bus, &request("text:second").body)
    });
    let (first_id, second_id) = (7001, 7002);
    assert_eq!(text(&arguments[0]), "Bientôt");
    assert_eq!(text(&arguments[2]), "dev.bientot.Bientot");
    assert_eq!(text(&arguments[3]), first.title);
    assert_eq!(
        <Vec<String>>::try_from(arguments[5].try_clone().unwrap()).unwrap(),
        ["default", ""]
    );
    let hints = <HashMap<String, OwnedValue>>::try_from(arguments[6].try_clone().unwrap()).unwrap();
    assert_eq!(hints.keys().collect::<Vec<_>>(), ["desktop-entry"]);
    assert_eq!(text(&hints["desktop-entry"]), "dev.bientot.Bientot");
    assert_eq!(i32::try_from(&arguments[7]).unwrap(), -1);

    server_clicks(&bus, second_id, "default");
    server_clicks(&bus, first_id, "default");
    assert_eq!(clicks.recv_timeout(WAIT), Ok(Some("text:second".into())));
    assert_eq!(clicks.recv_timeout(WAIT), Ok(Some("text:first".into())));

    server_clicks(&bus, second_id, "snooze");
    server_clicks(&bus, second_id + 1000, "default");
    emit(
        &bus,
        SERVER,
        SERVER_PATH,
        SERVER,
        "NotificationClosed",
        "uu",
        vec![first_id.into(), 2u32.into()],
    );
    server_clicks(&bus, first_id, "default");
    assert_eq!(
        clicks.recv_timeout(QUIET),
        Err(mpsc::RecvTimeoutError::Timeout),
        "another action, another application's notification, and a closed notification are no clicks"
    );
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_reports_a_server_that_does_not_answer_as_unavailable() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let bus = session();

    let refused = {
        let _refusing = Refusing::new(&bus, "GetServerInformation");
        started(Service::Server).map(|_| ())
    };

    let error = refused.expect_err("a server that refuses GetServerInformation is unavailable");
    assert!(error.to_string().starts_with(SERVER), "{error}");
    assert!(started(Service::Server).is_ok(), "the server answers again");
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_keeps_showing_after_the_server_refused_one() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let bus = session();
    let (notifications, _clicks) = started(Service::Server).unwrap();
    let refused = request("refused:first");

    {
        let _refusing = Refusing::new(&bus, "Notify");
        notifications.show(&refused).unwrap();
        eventually("the refused call", || {
            calls(&bus, SERVER, SERVER_PATH, "Notify")
                .iter()
                .any(|arguments| text(&arguments[4]) == refused.body)
                .then_some(())
        });
    }
    notifications.show(&request("refused:next")).unwrap();

    eventually("the next notification", || {
        notified(&bus, &request("refused:next").body)
    });
}

/// A notification portal on the session bus for the life of the value: python-dbusmock serving
/// the portal's interface with an `AddNotification` that keeps its calls.
struct Portal {
    mock: Child,
}

impl Portal {
    fn start(bus: &Connection) -> Self {
        let python = std::env::var("HARNESS_PYTHON").unwrap_or_else(|_| "/usr/bin/python3".into());
        let mock = Command::new(python)
            .args([
                "-m",
                "dbusmock",
                "--session",
                PORTAL_DESKTOP,
                PORTAL_PATH,
                PORTAL,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("python-dbusmock starts");
        let portal = Self { mock };
        eventually("the portal mock", || {
            async_io::block_on(crate::dbus::has_owner(bus, PORTAL_DESKTOP)).then_some(())
        });
        mock_call::<_, ()>(
            bus,
            PORTAL_DESKTOP,
            PORTAL_PATH,
            "AddProperty",
            &(PORTAL, "version", Value::U32(2)),
        );
        mock_call::<_, ()>(
            bus,
            PORTAL_DESKTOP,
            PORTAL_PATH,
            "AddMethod",
            &(PORTAL, "AddNotification", "sa{sv}", "", ""),
        );
        portal
    }
}

impl Drop for Portal {
    fn drop(&mut self) {
        let _ = self.mock.kill();
        let _ = self.mock.wait();
    }
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_shows_through_the_portal_and_reports_the_clicks_on_the_body() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let bus = session();
    assert!(
        started(Service::Portal).is_err(),
        "no portal is unavailable"
    );
    let _portal = Portal::start(&bus);
    let (notifications, clicks) = started(Service::Portal).unwrap();
    let shown = request("portal:first");

    notifications.show(&shown).unwrap();

    let arguments = eventually("the portal's notification", || {
        calls(&bus, PORTAL_DESKTOP, PORTAL_PATH, "AddNotification").pop()
    });
    assert_eq!(text(&arguments[0]), "portal:first");
    let notification =
        <HashMap<String, OwnedValue>>::try_from(arguments[1].try_clone().unwrap()).unwrap();
    let mut keys: Vec<_> = notification.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["body", "default-action", "title"]);
    assert_eq!(text(&notification["title"]), shown.title);
    assert_eq!(text(&notification["body"]), shown.body);
    assert_eq!(text(&notification["default-action"]), "open");

    let invoked = |id: &str, action: &str| {
        let parameter = Array::new(&zbus::zvariant::Signature::Variant);
        emit(
            &bus,
            PORTAL_DESKTOP,
            PORTAL_PATH,
            PORTAL,
            "ActionInvoked",
            "ssav",
            vec![id.into(), action.into(), parameter.into()],
        );
    };
    invoked("another-application", "open");
    invoked("portal:first", "dismiss");
    invoked("portal:first", "open");
    assert_eq!(clicks.recv_timeout(WAIT), Ok(Some("portal:first".into())));
    assert_eq!(
        clicks.recv_timeout(QUIET),
        Err(mpsc::RecvTimeoutError::Timeout)
    );
}
