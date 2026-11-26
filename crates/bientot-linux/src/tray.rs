//! The tray icon as a StatusNotifierItem (research.md, R4; contracts/linux-backends.md, « Tray »).
//!
//! A host draws the item from what it reads on D-Bus: ARGB32 pixmaps, a tooltip, and a dbusmenu
//! tree. [`menu`] and [`pixmaps`] turn what the app publishes into those; [`click_position`]
//! reads the position an `Activate` carries. [`HostWatch`] says whether a host shows the item:
//! all a client can see of the host is the watcher, `org.kde.StatusNotifierWatcher`, since the
//! watchers of KDE and GNOME answer « a host is registered » whether one is or not. On Linux,
//! `StatusNotifier` serves the item with `ksni`.

#[cfg(target_os = "linux")]
mod native;

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use bientot_platform::tray::{MenuEntry, ScreenPoint, TrayHost, TrayIcon, item_id};

#[cfg(target_os = "linux")]
pub use native::StatusNotifier;

/// The sizes of the pixmaps the item carries: the host scales the nearest to its panel.
pub const ICON_SIZES: [u32; 5] = [16, 22, 24, 32, 48];

/// How long the watcher may be gone before the app is told that no host shows the item, so that
/// a shell that restarts does not switch the app to its window (data-model.md, `TrayHost`).
pub const HOST_GRACE: Duration = Duration::from_secs(5);

/// An entry of the item's dbusmenu, as the host shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuNode {
    Item {
        /// The label as dbusmenu reads it: an underscore marks the access key, two stand for one.
        label: String,
        enabled: bool,
        /// The [`item_id`] the app is told when the item is clicked.
        id: String,
    },
    Submenu {
        label: String,
        children: Vec<MenuNode>,
    },
    Separator,
}

/// The dbusmenu of `entries`, the menu of `generation`.
pub fn menu<C>(entries: &[MenuEntry<C>], generation: u64) -> Vec<MenuNode> {
    nodes(entries, generation, &[])
}

fn nodes<C>(entries: &[MenuEntry<C>], generation: u64, path: &[usize]) -> Vec<MenuNode> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let path = [path, &[index]].concat();
            match entry {
                MenuEntry::Item { label, enabled, .. } => MenuNode::Item {
                    label: dbusmenu_label(label),
                    enabled: *enabled,
                    id: item_id(generation, &path),
                },
                MenuEntry::Submenu { label, items } => MenuNode::Submenu {
                    label: dbusmenu_label(label),
                    children: nodes(items, generation, &path),
                },
                MenuEntry::Separator => MenuNode::Separator,
            }
        })
        .collect()
}

/// `label` shown as written: Bientôt's labels have no access key.
fn dbusmenu_label(label: &str) -> String {
    label.replace('_', "__")
}

/// One size of the icon, as `IconPixmap` carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pixmap {
    /// Width and height, in pixels.
    pub size: u32,
    /// ARGB32 in network byte order: alpha, red, green, blue, one byte each, row by row.
    pub argb: Vec<u8>,
}

/// The pixmaps of `icon`, whose buffers are straight RGBA.
pub fn pixmaps(icon: TrayIcon<'_>) -> Vec<Pixmap> {
    icon.pixmaps
        .iter()
        .map(|&(size, rgba)| Pixmap {
            size,
            argb: rgba
                .chunks_exact(4)
                .flat_map(|pixel| [pixel[3], pixel[0], pixel[1], pixel[2]])
                .collect(),
        })
        .collect()
}

/// The position of a click from the `x` and `y` of `Activate`. A host that knows none sends
/// 0, 0. The hosts tried send the pointer's position, waybar on Wayland included (research.md,
/// « Spike results », A-5), so a Wayland session keeps it too.
pub fn click_position(x: i32, y: i32) -> Option<ScreenPoint> {
    (x, y).ne(&(0, 0)).then_some(ScreenPoint { x, y })
}

/// What the item hears of the watcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatcherSignal {
    /// The watcher came back, and the item registered with it again.
    Online,
    /// The watcher left the bus, or the item could not register with it.
    Offline,
}

/// Whether the app is told that a host shows the item. A watcher that leaves counts as no host
/// only once it has stayed away for the grace; one that comes back counts at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostWatch {
    told: TrayHost,
    /// When the watcher left, while the app is still told that a host is present.
    left_at: Option<Instant>,
    grace: Duration,
}

impl HostWatch {
    /// The app was told `host` when the item started.
    pub fn new(host: TrayHost, grace: Duration) -> Self {
        Self {
            told: host,
            left_at: None,
            grace,
        }
    }

    /// What the app was last told.
    pub fn told(&self) -> TrayHost {
        self.told
    }

    /// Hears `signal` at `now`; what the app must be told now, if anything.
    pub fn heard(&mut self, signal: WatcherSignal, now: Instant) -> Option<TrayHost> {
        match (signal, self.told) {
            (WatcherSignal::Online, TrayHost::Present) => {
                self.left_at = None;
                None
            }
            (WatcherSignal::Online, TrayHost::Absent) => self.tell(TrayHost::Present),
            (WatcherSignal::Offline, TrayHost::Present) => {
                self.left_at.get_or_insert(now);
                None
            }
            (WatcherSignal::Offline, TrayHost::Absent) => None,
        }
    }

    /// When the grace of a watcher that left runs out.
    pub fn deadline(&self) -> Option<Instant> {
        self.left_at.map(|left_at| left_at + self.grace)
    }

    /// What the app must be told at `now`, if anything: that no host shows the item, once the
    /// grace ran out.
    pub fn elapsed(&mut self, now: Instant) -> Option<TrayHost> {
        if self.deadline().is_some_and(|deadline| deadline <= now) {
            self.tell(TrayHost::Absent)
        } else {
            None
        }
    }

    fn tell(&mut self, host: TrayHost) -> Option<TrayHost> {
        self.left_at = None;
        self.told = host;
        Some(host)
    }
}

/// Follows the watcher's `signals` until they end, handing `changed` each change the app must be
/// told, as soon as it is due.
pub fn follow(
    mut watch: HostWatch,
    signals: &Receiver<WatcherSignal>,
    mut changed: impl FnMut(TrayHost),
) {
    loop {
        let signal = match watch.deadline() {
            Some(deadline) => {
                match signals.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(signal) => Some(signal),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            None => match signals.recv() {
                Ok(signal) => Some(signal),
                Err(_) => return,
            },
        };
        let now = Instant::now();
        let told = match signal {
            Some(signal) => watch.heard(signal, now),
            None => watch.elapsed(now),
        };
        if let Some(host) = told {
            changed(host);
        }
    }
}

#[cfg(test)]
mod tests;
