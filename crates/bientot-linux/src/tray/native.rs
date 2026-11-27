//! The item on the session bus, served by `ksni` on a thread of its own, the thread that hands
//! it what the app publishes, and the thread that follows its watcher.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use bientot_platform::system::InstallKind;
use bientot_platform::tray::{Tray, TrayEvent, TrayHost, TrayState};
use ksni::blocking::{Handle, TrayMethods as _};
use ksni::menu::{StandardItem, SubMenu};
use ksni::{MenuItem, OfflineReason, Status, ToolTip};

use super::{
    HOST_GRACE, HostWatch, ICON_SIZES, Latest, MenuNode, WatcherSignal, click_position, follow,
    menu, owns_a_bus_name, pixmaps,
};

/// The item's `Id`: the application id the desktop files and the Flatpak use.
const ID: &str = "dev.bientot.Bientot";
const TITLE: &str = "Bientôt";

type Events = Arc<dyn Fn(TrayEvent) + Send + Sync>;

/// The tray icon as a StatusNotifierItem. Its clicks, the menu items chosen and the host's
/// changes reach the `events` it was made with, from threads of its own.
pub struct StatusNotifier {
    /// `None` when the item could not be served: no session bus, or a watcher that refused it.
    handle: Option<Handle<Item>>,
    published: Arc<Latest<Shown>>,
    host: Arc<Mutex<TrayHost>>,
}

/// The item as `ksni` reads it.
struct Item {
    /// `None` before the first [`Tray::publish`].
    shown: Option<Shown>,
    events: Events,
    watcher: Sender<WatcherSignal>,
}

/// What the app published.
struct Shown {
    pixmaps: Vec<ksni::Icon>,
    tooltip: String,
    menu: Vec<MenuNode>,
}

impl StatusNotifier {
    /// Serves the item of an install of `kind` on the session bus, with a well-known name when
    /// [`owns_a_bus_name`] says so. It shows nothing before the first [`Tray::publish`].
    pub fn new(kind: InstallKind, events: impl Fn(TrayEvent) + Send + Sync + 'static) -> Self {
        let (watcher, signals) = mpsc::channel();
        let item = Item {
            shown: None,
            events: Arc::new(events),
            watcher,
        };
        let events = Arc::clone(&item.events);
        // Without `assume_sni_available`, a watcher missing at start fails `spawn` for good, and
        // the item would not register when one appears.
        let served = item
            .disable_dbus_name(!owns_a_bus_name(kind))
            .assume_sni_available(true)
            .spawn();
        let published = Arc::new(Latest::default());
        let Ok(handle) = served else {
            return Self {
                handle: None,
                published,
                host: Arc::new(Mutex::new(TrayHost::Absent)),
            };
        };
        // `spawn` calls `watcher_offline` before it returns when no watcher owns its name.
        let host = match signals.try_recv() {
            Ok(WatcherSignal::Offline) => TrayHost::Absent,
            _ => TrayHost::Present,
        };
        let shared = Arc::new(Mutex::new(host));
        let told = Arc::clone(&shared);
        thread::Builder::new()
            .name("tray-host".into())
            .spawn(move || {
                follow(HostWatch::new(host, HOST_GRACE), &signals, |host| {
                    *told.lock().unwrap_or_else(PoisonError::into_inner) = host;
                    events(TrayEvent::HostChanged(host));
                });
            })
            .expect("the system starts a thread");
        let taken = Arc::clone(&published);
        let updated = handle.clone();
        thread::Builder::new()
            .name("tray-publish".into())
            .spawn(move || {
                while let Some(shown) = taken.take() {
                    updated.update(|item| item.shown = Some(shown));
                }
            })
            .expect("the system starts a thread");
        Self {
            handle: Some(handle),
            published,
            host: shared,
        }
    }

    /// The backend's name in the diagnostic log: `status-notifier` while the item is served,
    /// `none` when it could not be.
    pub fn backend(&self) -> &'static str {
        match self.handle {
            Some(_) => "status-notifier",
            None => "none",
        }
    }
}

impl Drop for StatusNotifier {
    /// Takes the item off the bus, without waiting: a D-Bus call under way could hold the
    /// app's exit for its whole timeout.
    fn drop(&mut self) {
        self.published.close();
        if let Some(handle) = &self.handle {
            drop(handle.shutdown());
        }
    }
}

impl<C> Tray<C> for StatusNotifier {
    fn icon_sizes(&self) -> Vec<u32> {
        ICON_SIZES.to_vec()
    }

    /// No interface tells the colour of the panel (T053).
    fn panel_is_dark(&self) -> Option<bool> {
        None
    }

    fn host(&self) -> TrayHost {
        *self.host.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Returns at once: the `tray-publish` thread hands `state` to `ksni`, whose update waits as
    /// long as a watcher that came back takes to answer the item's registration. A state not
    /// handed yet gives way to the next. `ksni` sends the host only the properties that changed,
    /// and the menu items that changed.
    fn publish(&self, state: TrayState<'_, C>) {
        if self.handle.is_none() {
            return;
        }
        let pixmaps = pixmaps(state.icon)
            .into_iter()
            .map(|pixmap| ksni::Icon {
                width: pixmap.size as i32,
                height: pixmap.size as i32,
                data: pixmap.argb,
            })
            .collect();
        self.published.hand(Shown {
            pixmaps,
            tooltip: state.tooltip.to_owned(),
            menu: menu(state.menu, state.generation),
        });
    }
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        ID.into()
    }

    fn title(&self) -> String {
        TITLE.into()
    }

    /// Hosts hide a passive item, so the icon shows only once the app published the day.
    fn status(&self) -> Status {
        if self.shown.is_some() {
            Status::Active
        } else {
            Status::Passive
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.shown
            .as_ref()
            .map(|shown| shown.pixmaps.clone())
            .unwrap_or_default()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: self
                .shown
                .as_ref()
                .map(|shown| shown.tooltip.clone())
                .unwrap_or_default(),
            ..ToolTip::default()
        }
    }

    fn activate(&mut self, x: i32, y: i32) {
        (self.events)(TrayEvent::Activated {
            at: click_position(x, y),
        });
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        self.shown
            .iter()
            .flat_map(|shown| &shown.menu)
            .map(menu_item)
            .collect()
    }

    fn watcher_online(&self) {
        self.watcher.send(WatcherSignal::Online).ok();
    }

    fn watcher_offline(&self, _reason: OfflineReason) -> bool {
        self.watcher.send(WatcherSignal::Offline).ok();
        // Keeps listening: the item registers again when a watcher appears.
        true
    }
}

fn menu_item(node: &MenuNode) -> MenuItem<Item> {
    match node {
        MenuNode::Item { label, enabled, id } => {
            let id = id.clone();
            StandardItem {
                label: label.clone(),
                enabled: *enabled,
                activate: Box::new(move |item: &mut Item| {
                    (item.events)(TrayEvent::MenuChosen { id: id.clone() });
                }),
                ..StandardItem::default()
            }
            .into()
        }
        MenuNode::Submenu { label, children } => SubMenu {
            label: label.clone(),
            submenu: children.iter().map(menu_item).collect(),
            ..SubMenu::default()
        }
        .into(),
        MenuNode::Separator => MenuItem::Separator,
    }
}

#[cfg(test)]
mod tests;
