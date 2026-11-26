//! The item on the session bus, served by `ksni` on a thread of its own, and the thread that
//! follows its watcher.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use bientot_platform::tray::{Tray, TrayEvent, TrayHost, TrayState};
use ksni::blocking::{Handle, TrayMethods as _};
use ksni::menu::{StandardItem, SubMenu};
use ksni::{MenuItem, OfflineReason, Status, ToolTip};

use super::{
    HOST_GRACE, HostWatch, ICON_SIZES, MenuNode, WatcherSignal, click_position, follow, menu,
    pixmaps,
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
    host: Arc<Mutex<TrayHost>>,
}

/// What the item shows, as `ksni` reads it.
struct Item {
    published: bool,
    pixmaps: Vec<ksni::Icon>,
    tooltip: String,
    menu: Vec<MenuNode>,
    events: Events,
    watcher: Sender<WatcherSignal>,
}

impl StatusNotifier {
    /// Serves the item on the session bus. It shows nothing before the first
    /// [`Tray::publish`]. Outside a Flatpak it owns the well-known name the specification asks
    /// for; inside, the sandbox forbids it and current hosts do without (A-6).
    pub fn new(events: impl Fn(TrayEvent) + Send + Sync + 'static) -> Self {
        let (watcher, signals) = mpsc::channel();
        let item = Item {
            published: false,
            pixmaps: Vec::new(),
            tooltip: String::new(),
            menu: Vec::new(),
            events: Arc::new(events),
            watcher,
        };
        let events = Arc::clone(&item.events);
        // Without `assume_sni_available`, a watcher missing at start fails `spawn` for good, and
        // the item would not register when one appears.
        let served = item
            .disable_dbus_name(std::env::var_os("FLATPAK_ID").is_some())
            .assume_sni_available(true)
            .spawn();
        let Ok(handle) = served else {
            return Self {
                handle: None,
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
        Self {
            handle: Some(handle),
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

    /// `ksni` sends the host only the properties that changed, and the menu items that changed.
    fn publish(&self, state: TrayState<'_, C>) {
        let Some(handle) = &self.handle else {
            return;
        };
        let pixmaps = pixmaps(state.icon)
            .into_iter()
            .map(|pixmap| ksni::Icon {
                width: pixmap.size as i32,
                height: pixmap.size as i32,
                data: pixmap.argb,
            })
            .collect();
        let tooltip = state.tooltip.to_owned();
        let menu = menu(state.menu, state.generation);
        handle.update(move |item| {
            item.published = true;
            item.pixmaps = pixmaps;
            item.tooltip = tooltip;
            item.menu = menu;
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
        if self.published {
            Status::Active
        } else {
            Status::Passive
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.pixmaps.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: self.tooltip.clone(),
            ..ToolTip::default()
        }
    }

    fn activate(&mut self, x: i32, y: i32) {
        (self.events)(TrayEvent::Activated {
            at: click_position(x, y),
        });
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        self.menu.iter().map(menu_item).collect()
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
