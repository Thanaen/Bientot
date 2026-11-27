//! The notification-area icon: its menu as every tray shows it (labels, separators and submenus,
//! each item carrying the command the app runs when it is chosen, and the ids that name the items
//! to the system), and the port that shows it.

use std::cell::RefCell;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry<C> {
    Item {
        label: String,
        command: C,
        enabled: bool,
    },
    Submenu {
        label: String,
        items: Vec<MenuEntry<C>>,
    },
    Separator,
}

/// The id of the item at `path` in the menu of `generation`: `3:4.1` names the second item of
/// the submenu of the fifth entry. A new menu gets a new generation, so that a click on an item
/// of a menu since replaced names no item.
pub fn item_id(generation: u64, path: &[usize]) -> String {
    let path: Vec<String> = path.iter().map(usize::to_string).collect();
    format!("{generation}:{}", path.join("."))
}

/// What the item `id` does, when it names an enabled item of `entries`, the menu of
/// `generation`.
pub fn chosen<C: Clone>(entries: &[MenuEntry<C>], generation: u64, id: &str) -> Option<C> {
    let (shown, path) = id.split_once(':')?;
    if shown != generation.to_string() {
        return None;
    }
    let mut indices = path.split('.').map(|index| index.parse::<usize>().ok());
    let mut entries = entries;
    loop {
        match entries.get(indices.next()??)? {
            MenuEntry::Submenu { items, .. } => entries = items,
            MenuEntry::Item {
                command,
                enabled: true,
                ..
            } => return indices.next().is_none().then(|| command.clone()),
            MenuEntry::Item { .. } | MenuEntry::Separator => return None,
        }
    }
}

/// The icon in every size the host asked for: straight RGBA, one buffer per size in pixels.
#[derive(Clone, Copy, Debug)]
pub struct TrayIcon<'a> {
    pub pixmaps: &'a [(u32, &'a [u8])],
}

/// What the tray shows.
#[derive(Clone, Copy, Debug)]
pub struct TrayState<'a, C> {
    pub icon: TrayIcon<'a>,
    pub tooltip: &'a str,
    pub menu: &'a [MenuEntry<C>],
    /// The generation of `menu`, which its item ids name.
    pub generation: u64,
}

/// Whether something on the desktop shows tray icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrayHost {
    Present,
    Absent,
}

/// A point on the desktop, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScreenPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// A primary click. `at` is `None` when the host gave no usable position.
    Activated {
        at: Option<ScreenPoint>,
    },
    /// The menu item of this [`item_id`] was chosen.
    MenuChosen {
        id: String,
    },
    HostChanged(TrayHost),
}

/// The notification-area icon, whose menu items run commands of type `C`. Its events arrive on
/// the channel the composition root builds with it.
pub trait Tray<C> {
    /// The sizes the host wants, in pixels, most wanted first.
    fn icon_sizes(&self) -> Vec<u32>;

    /// Whether the panel is dark, when the platform can tell.
    fn panel_is_dark(&self) -> Option<bool>;

    fn host(&self) -> TrayHost;

    fn publish(&self, state: TrayState<'_, C>);
}

/// What an [`InMemoryTray`] was last given to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Published<C> {
    /// Each pixmap of the icon: its size and its RGBA rows.
    pub pixmaps: Vec<(u32, Vec<u8>)>,
    pub tooltip: String,
    pub menu: Vec<MenuEntry<C>>,
    pub generation: u64,
}

/// A tray that answers what it was given and keeps what it was last given to show.
pub struct InMemoryTray<C> {
    icon_sizes: Vec<u32>,
    panel_is_dark: Option<bool>,
    host: TrayHost,
    published: RefCell<Option<Published<C>>>,
}

impl<C: Clone> InMemoryTray<C> {
    pub fn new(icon_sizes: Vec<u32>, panel_is_dark: Option<bool>, host: TrayHost) -> Self {
        Self {
            icon_sizes,
            panel_is_dark,
            host,
            published: RefCell::new(None),
        }
    }

    pub fn published(&self) -> Option<Published<C>> {
        self.published.borrow().clone()
    }
}

impl<C: Clone> Tray<C> for InMemoryTray<C> {
    fn icon_sizes(&self) -> Vec<u32> {
        self.icon_sizes.clone()
    }

    fn panel_is_dark(&self) -> Option<bool> {
        self.panel_is_dark
    }

    fn host(&self) -> TrayHost {
        self.host
    }

    fn publish(&self, state: TrayState<'_, C>) {
        *self.published.borrow_mut() = Some(Published {
            pixmaps: state
                .icon
                .pixmaps
                .iter()
                .map(|(size, rgba)| (*size, rgba.to_vec()))
                .collect(),
            tooltip: state.tooltip.to_owned(),
            menu: state.menu.to_vec(),
            generation: state.generation,
        });
    }
}
