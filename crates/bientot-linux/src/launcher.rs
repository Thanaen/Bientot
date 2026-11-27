//! The AppImage in the desktop's menu of applications (contracts/packaging.md,
//! « Self-integration »). An AppImage has no installer, so Bientôt writes its own entry,
//! `$XDG_DATA_HOME/applications/dev.bientot.Bientot.desktop`, which starts the AppImage file, and
//! the icon the entry names, `$XDG_DATA_HOME/icons/hicolor/256x256/apps/dev.bientot.Bientot.png`,
//! copied from the `.DirIcon` of the mounted AppImage. The entry inside the AppImage keeps vpk's
//! names. Desktops watch these folders, and the entry names no MIME type, so no cache needs
//! rebuilding.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use bientot_platform::launcher::LauncherPort;
use bientot_platform::system::InstallKind;

use crate::APP_ID;
use crate::desktop_entry::{self, Entry, base_directory, escape_value, exec_value, write};

/// The launcher entry and icon of one AppImage file.
#[derive(Debug)]
pub struct LauncherEntry {
    entry: PathBuf,
    icon: PathBuf,
    /// The AppImage file, absolute and UTF-8: what the entry starts.
    appimage: String,
    /// The icon the AppImage holds, in its mount; `None` when the runtime names no mount.
    dir_icon: Option<PathBuf>,
}

impl LauncherEntry {
    /// The entry under `data_home`, the user's data folder, of the AppImage `appimage` mounted at
    /// `appdir`. `None` when `appimage` is not an absolute UTF-8 path, which no entry can start.
    pub fn new(data_home: &Path, appimage: &Path, appdir: Option<&Path>) -> Option<Self> {
        let appimage = appimage
            .to_str()
            .filter(|path| crate::is_absolute(Path::new(path)))?;
        Some(Self {
            entry: data_home
                .join("applications")
                .join(format!("{APP_ID}.desktop")),
            icon: data_home
                .join("icons/hicolor/256x256/apps")
                .join(format!("{APP_ID}.png")),
            appimage: appimage.to_owned(),
            dir_icon: appdir
                .filter(|path| crate::is_absolute(path))
                .map(|appdir| appdir.join(".DirIcon")),
        })
    }

    /// The entry of this process when it runs as an AppImage. `None` for any other install, a
    /// Flatpak or a build run where it was made, which has its entry from elsewhere or none.
    pub fn for_this_process() -> Option<Self> {
        Self::of(
            |name| std::env::var_os(name),
            Path::new("/.flatpak-info").exists(),
        )
    }

    /// The entry of the AppImage an environment that `variable` reads names: `$APPIMAGE`, mounted
    /// at `$APPDIR` as the AppImage runtime sets them, under `$XDG_DATA_HOME`, else
    /// `~/.local/share`.
    fn of(variable: impl Fn(&str) -> Option<OsString>, flatpak_info: bool) -> Option<Self> {
        if crate::system::install_kind(&variable, flatpak_info) != InstallKind::AppImage {
            return None;
        }
        let data_home = base_directory(&variable, "XDG_DATA_HOME", ".local/share")?;
        let appimage = PathBuf::from(variable("APPIMAGE")?);
        let appdir = variable("APPDIR").map(PathBuf::from);
        Self::new(&data_home, &appimage, appdir.as_deref())
    }

    fn read(&self) -> io::Result<Option<Entry>> {
        desktop_entry::read(&self.entry)
    }

    /// Writes the icon, then the entry that starts this AppImage: the menu shows an entry as
    /// soon as it appears.
    fn add(&self) -> io::Result<()> {
        if let Some(dir_icon) = &self.dir_icon {
            write(&self.icon, &std::fs::read(dir_icon)?)?;
        }
        write(&self.entry, desktop_file(&self.appimage).as_bytes())
    }

    /// Removes the entry and the icon, unless the entry starts another AppImage, whose icon it is
    /// too.
    fn remove(&self) -> io::Result<()> {
        if self
            .read()?
            .is_some_and(|entry| !entry.starts(&self.appimage))
        {
            return Ok(());
        }
        desktop_entry::remove(&self.entry)?;
        desktop_entry::remove(&self.icon)
    }
}

impl LauncherPort for LauncherEntry {
    /// An entry made `Hidden`, which the specification reads as deleted, is not in the menu.
    fn is_added(&self) -> bool {
        self.read()
            .ok()
            .flatten()
            .is_some_and(|entry| !entry.turned_off && entry.starts(&self.appimage))
    }

    fn set_added(&self, added: bool) -> io::Result<()> {
        if added { self.add() } else { self.remove() }
    }

    /// An entry that starts this AppImage is left as it is, with what the user's menu editor
    /// changed in it, and so is a `Hidden` one, which the user deleted wherever it pointed.
    fn settle(&self, first_run: bool) -> io::Result<()> {
        match self.read()? {
            None if first_run => self.add(),
            Some(entry) if !entry.turned_off && !entry.starts(&self.appimage) => self.add(),
            None | Some(_) => Ok(()),
        }
    }
}

/// The entry that starts the AppImage `appimage`, as Bientôt writes it, with the name,
/// categories and window class of `contracts/packaging.md`. `TryExec` lets the menu leave the
/// entry out once the file is gone, until the AppImage starts again from where it moved.
fn desktop_file(appimage: &str) -> String {
    let exec = exec_value(appimage, "");
    let try_exec = escape_value(appimage);
    format!(
        "[Desktop Entry]\nType=Application\nName=Bientôt\nExec={exec}\nTryExec={try_exec}\n\
         Icon={APP_ID}\nCategories=Office;Utility;\nStartupWMClass={APP_ID}\n"
    )
}

#[cfg(test)]
mod tests;
