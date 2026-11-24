//! The menu of the notification-area icon as every tray shows it: labels, separators and
//! submenus, each item carrying the command the app runs when it is chosen, and the ids that
//! name the items to the system.

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
