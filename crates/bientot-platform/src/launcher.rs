//! The port of the entry that shows Bientôt in the desktop's menu of applications, for a copy
//! that writes its own: an AppImage has no installer to add it (specs/005-linux-support, US5).

use std::io;

/// This copy's entry in the menu of applications. Reading and changing it touches files, so the
/// window calls it off its thread.
pub trait LauncherPort: Send + Sync {
    /// Whether the menu shows this copy: its entry is there and starts it. An entry that cannot
    /// be read reads as absent.
    fn is_added(&self) -> bool;

    /// Adds this copy's entry and icon to the menu, or removes them.
    fn set_added(&self, added: bool) -> io::Result<()>;

    /// What each start does. On the copy's first run (`first_run`), adds the entry. Later, points
    /// an entry that starts another path at this copy, as after the file moved, and leaves a
    /// removed entry removed.
    fn settle(&self, first_run: bool) -> io::Result<()>;
}
