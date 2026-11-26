//! Against the StatusNotifierWatcher mock that `scripts/linux-desktop-harness.sh` starts on the
//! session bus, which the tests share, so they take turns. Each test plays the host: it reads the
//! item's properties and menu, and clicks it, over D-Bus.

use std::collections::BTreeSet;
use std::sync::PoisonError;
use std::thread::sleep;
use std::time::{Duration, Instant};

use bientot_platform::tray::{MenuEntry, ScreenPoint, TrayIcon};
use zbus::Connection;
use zbus::zvariant::{OwnedValue, Structure, Value};

use super::*;
use crate::dbus::{BUS, BUS_PATH, call};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const DBUSMENU: &str = "com.canonical.dbusmenu";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const MOCK: &str = "org.freedesktop.DBus.Mock";
const WAIT: Duration = Duration::from_secs(5);

fn session() -> Connection {
    async_io::block_on(Connection::session()).unwrap()
}

fn property(
    bus: &Connection,
    destination: &str,
    path: &str,
    interface: &str,
    name: &str,
) -> OwnedValue {
    async_io::block_on(call(
        bus,
        destination,
        path,
        PROPERTIES,
        "Get",
        &(interface, name),
    ))
    .unwrap_or_else(|error| panic!("cannot read {name} of {destination}: {error}"))
}

/// The items the watcher lists, as `<bus name>/StatusNotifierItem`.
fn registered(bus: &Connection) -> BTreeSet<String> {
    let items = property(
        bus,
        WATCHER,
        WATCHER_PATH,
        WATCHER,
        "RegisteredStatusNotifierItems",
    );
    <Vec<String>>::try_from(items)
        .unwrap()
        .into_iter()
        .collect()
}

/// The item the watcher lists now and did not list in `before`.
fn new_item(bus: &Connection, before: &BTreeSet<String>) -> Seen {
    let added: Vec<String> = registered(bus).difference(before).cloned().collect();
    let [listed] = added.as_slice() else {
        panic!("one item registered, not {added:?}");
    };
    let (destination, path) = listed.split_at(listed.find('/').unwrap());
    Seen {
        listed: listed.clone(),
        destination: destination.into(),
        path: path.into(),
    }
}

/// Waits up to [`WAIT`] for `condition`.
fn eventually(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !condition() {
        assert!(Instant::now() < deadline, "gave up waiting: {what}");
        sleep(Duration::from_millis(50));
    }
}

/// The item as a host sees it.
struct Seen {
    listed: String,
    destination: String,
    path: String,
}

impl Seen {
    fn property(&self, bus: &Connection, name: &str) -> OwnedValue {
        property(bus, &self.destination, &self.path, ITEM, name)
    }

    fn text(&self, bus: &Connection, name: &str) -> String {
        String::try_from(self.property(bus, name)).unwrap()
    }

    /// Each pixmap's width, height and bytes.
    fn pixmaps(&self, bus: &Connection) -> Vec<(i32, i32, Vec<u8>)> {
        <Vec<(i32, i32, Vec<u8>)>>::try_from(self.property(bus, "IconPixmap")).unwrap()
    }

    fn tooltip_title(&self, bus: &Connection) -> String {
        let tooltip = self.property(bus, "ToolTip");
        let Value::Structure(tooltip) = &*tooltip else {
            panic!("the tooltip is a structure: {tooltip:?}");
        };
        String::try_from(tooltip.fields()[2].try_clone().unwrap()).unwrap()
    }

    fn activate(&self, bus: &Connection, x: i32, y: i32) {
        async_io::block_on(call::<_, ()>(
            bus,
            &self.destination,
            &self.path,
            ITEM,
            "Activate",
            &(x, y),
        ))
        .unwrap();
    }

    /// The dbusmenu id of the entry labelled `label`.
    fn menu_id(&self, bus: &Connection, label: &str) -> i32 {
        let reply = async_io::block_on(bus.call_method(
            Some(self.destination.as_str()),
            MENU_PATH,
            Some(DBUSMENU),
            "GetLayout",
            &(0, -1, Vec::<String>::new()),
        ))
        .unwrap();
        let body = reply.body();
        let body: Structure<'_> = body.deserialize().unwrap();
        let Value::Structure(root) = &body.fields()[1] else {
            panic!("the layout is a structure");
        };
        find(root, label).unwrap_or_else(|| panic!("no entry labelled {label}"))
    }

    fn click(&self, bus: &Connection, id: i32) {
        async_io::block_on(call::<_, ()>(
            bus,
            &self.destination,
            MENU_PATH,
            DBUSMENU,
            "Event",
            &(id, "clicked", Value::from(""), 0u32),
        ))
        .unwrap();
    }
}

/// The id of the node labelled `label` in the layout `node`, an `(ia{sv}av)`.
fn find(node: &Structure<'_>, label: &str) -> Option<i32> {
    let [
        Value::I32(id),
        Value::Dict(properties),
        Value::Array(children),
    ] = node.fields()
    else {
        panic!("a layout node is an (ia{{sv}}av): {node:?}");
    };
    let own: Option<String> = properties.get(&"label").unwrap();
    if own.as_deref() == Some(label) {
        return Some(*id);
    }
    children.iter().find_map(|child| match child {
        Value::Value(child) => match &**child {
            Value::Structure(child) => find(child, label),
            other => panic!("a child is a layout node: {other:?}"),
        },
        other => panic!("a child is a variant: {other:?}"),
    })
}

fn served() -> (StatusNotifier, mpsc::Receiver<TrayEvent>) {
    let (sender, events) = mpsc::channel();
    let tray = StatusNotifier::new(move |event| {
        sender.send(event).ok();
    });
    (tray, events)
}

fn host(tray: &StatusNotifier) -> TrayHost {
    Tray::<u8>::host(tray)
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_shows_the_published_icon_tooltip_and_menu_and_reports_the_clicks() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let bus = session();
    let before = registered(&bus);
    let (tray, events) = served();
    let item = new_item(&bus, &before);

    assert_eq!(tray.backend(), "status-notifier");
    assert_eq!(host(&tray), TrayHost::Present);
    assert_eq!(item.text(&bus, "Id"), "dev.bientot.Bientot");
    assert_eq!(item.text(&bus, "Title"), "Bientôt");
    assert_eq!(item.text(&bus, "Category"), "ApplicationStatus");
    assert_eq!(item.text(&bus, "Status"), "Passive");
    assert_eq!(item.pixmaps(&bus), []);

    let small = [0x10, 0x20, 0x30, 0xff].repeat(16 * 16);
    let large = [0xff, 0x00, 0x00, 0x80].repeat(22 * 22);
    let menu = [
        MenuEntry::Item {
            label: "Ouvrir Bientôt".into(),
            command: 0u8,
            enabled: true,
        },
        MenuEntry::Separator,
        MenuEntry::Submenu {
            label: "Plus".into(),
            items: vec![
                MenuEntry::Item {
                    label: "Exporter le diagnostic…".into(),
                    command: 1,
                    enabled: false,
                },
                MenuEntry::Item {
                    label: "Quitter".into(),
                    command: 2,
                    enabled: true,
                },
            ],
        },
    ];
    tray.publish(TrayState {
        icon: TrayIcon {
            pixmaps: &[(16, &small), (22, &large)],
        },
        tooltip: "Départ 17:55 · 5 h 04 min/7 h 00 min",
        menu: &menu,
        generation: 3,
    });

    eventually("the published item turns active", || {
        item.text(&bus, "Status") == "Active"
    });
    assert_eq!(
        item.tooltip_title(&bus),
        "Départ 17:55 · 5 h 04 min/7 h 00 min"
    );
    let pixmaps = item.pixmaps(&bus);
    let sizes: Vec<(i32, i32, usize)> = pixmaps
        .iter()
        .map(|(width, height, data)| (*width, *height, data.len()))
        .collect();
    assert_eq!(sizes, [(16, 16, 16 * 16 * 4), (22, 22, 22 * 22 * 4)]);
    assert_eq!(pixmaps[0].2[..4], [0xff, 0x10, 0x20, 0x30]);
    assert_eq!(pixmaps[1].2[..4], [0x80, 0xff, 0x00, 0x00]);

    item.click(&bus, item.menu_id(&bus, "Quitter"));
    assert_eq!(
        events.recv_timeout(WAIT).unwrap(),
        TrayEvent::MenuChosen { id: "3:2.1".into() }
    );

    item.activate(&bus, 1909, 1064);
    assert_eq!(
        events.recv_timeout(WAIT).unwrap(),
        TrayEvent::Activated {
            at: Some(ScreenPoint { x: 1909, y: 1064 })
        }
    );
    item.activate(&bus, 0, 0);
    assert_eq!(
        events.recv_timeout(WAIT).unwrap(),
        TrayEvent::Activated { at: None }
    );
}

/// The watcher mock: [`Watcher::leave`] takes its name off the bus, as a shell that restarts
/// does, and [`Watcher::stall`] makes it hold the registrations. Dropped, it gives the name back
/// and answers the registrations it holds.
struct Watcher {
    bus: Connection,
    /// The mock's own connection, which keeps answering while the name is gone.
    unique: String,
    away: bool,
    stalled: bool,
}

impl Watcher {
    fn new(bus: &Connection) -> Self {
        let unique =
            async_io::block_on(call(bus, BUS, BUS_PATH, BUS, "GetNameOwner", &(WATCHER,))).unwrap();
        Self {
            bus: bus.clone(),
            unique,
            away: false,
            stalled: false,
        }
    }

    fn mock<R>(&self, method: &str) -> R
    where
        R: for<'d> zbus::export::serde::Deserialize<'d> + zbus::zvariant::Type,
    {
        async_io::block_on(call(
            &self.bus,
            &self.unique,
            WATCHER_PATH,
            MOCK,
            method,
            &(),
        ))
        .unwrap_or_else(|error| panic!("the watcher mock refused {method}: {error}"))
    }

    fn leave(&mut self) {
        self.mock::<()>("Leave");
        self.away = true;
    }

    fn come_back(&mut self) {
        self.mock::<()>("Return");
        self.away = false;
    }

    fn stall(&mut self) {
        self.mock::<()>("Stall");
        self.stalled = true;
    }

    /// How many registrations the watcher holds.
    fn held(&self) -> u32 {
        self.mock("Held")
    }

    fn release(&mut self) {
        self.mock::<()>("Release");
        self.stalled = false;
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        if self.stalled {
            self.release();
        }
        if self.away {
            self.come_back();
        }
    }
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_tells_the_host_gone_only_after_the_grace_and_back_at_once() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let bus = session();
    let mut watcher = Watcher::new(&bus);
    let before = registered(&bus);
    let (tray, events) = served();
    let item = new_item(&bus, &before);

    watcher.leave();
    sleep(Duration::from_secs(1));
    watcher.come_back();
    eventually("the item registers with the watcher that came back", || {
        registered(&bus).contains(&item.listed)
    });
    assert_eq!(
        events.recv_timeout(HOST_GRACE + Duration::from_secs(1)),
        Err(mpsc::RecvTimeoutError::Timeout),
        "a watcher back within the grace changes nothing"
    );
    assert_eq!(host(&tray), TrayHost::Present);

    let left = Instant::now();
    watcher.leave();
    assert_eq!(
        events.recv_timeout(HOST_GRACE + WAIT).unwrap(),
        TrayEvent::HostChanged(TrayHost::Absent)
    );
    assert!(
        left.elapsed() >= HOST_GRACE,
        "told after {:?}",
        left.elapsed()
    );
    assert_eq!(host(&tray), TrayHost::Absent);

    watcher.come_back();
    assert_eq!(
        events.recv_timeout(WAIT).unwrap(),
        TrayEvent::HostChanged(TrayHost::Present)
    );
    assert_eq!(host(&tray), TrayHost::Present);
    eventually("the item registers again", || {
        registered(&bus).contains(&item.listed)
    });
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_publishes_at_once_while_the_watcher_holds_the_registration() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let bus = session();
    let mut watcher = Watcher::new(&bus);
    let before = registered(&bus);
    let (tray, _events) = served();
    let item = new_item(&bus, &before);

    watcher.leave();
    watcher.stall();
    watcher.come_back();
    eventually("the watcher holds the item's registration", || {
        watcher.held() == 1
    });
    let (done, returned) = mpsc::channel();
    let published = thread::scope(|scope| {
        scope.spawn(|| {
            for tooltip in ["Départ 17:55", "Départ 17:56"] {
                tray.publish(TrayState {
                    icon: TrayIcon { pixmaps: &[] },
                    tooltip,
                    menu: &[MenuEntry::<u8>::Separator],
                    generation: 1,
                });
            }
            done.send(()).unwrap();
        });
        let published = returned.recv_timeout(Duration::from_secs(1));
        watcher.release();
        published
    });

    assert_eq!(published, Ok(()), "publish waited on the watcher");
    eventually("the item shows the last state published", || {
        item.text(&bus, "Status") == "Active" && item.tooltip_title(&bus) == "Départ 17:56"
    });
    assert!(registered(&bus).contains(&item.listed));
}
