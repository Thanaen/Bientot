//! Start at sign-in outside a sandbox (research.md, R10; contracts/linux-backends.md, « Start at
//! sign-in »): an XDG autostart entry, `$XDG_CONFIG_HOME/autostart/dev.bientot.Bientot.desktop`,
//! whose `Exec` starts this install with [`AUTOSTART_ARGUMENT`]. The desktop starts every entry
//! of that folder as the user signs in, unless the entry is `Hidden` or
//! `X-GNOME-Autostart-enabled=false`.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use bientot_platform::startup::{AUTOSTART_ARGUMENT, StartupPort, StartupRegistrationState};
use bientot_platform::system::InstallKind;

use crate::APP_ID;
use crate::desktop_entry::{Entry, base_directory, exec_value, read, write};

/// What registers an install to start at sign-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Autostart {
    /// An entry in the user's autostart folder: [`XdgAutostart`].
    Entry,
    /// `org.freedesktop.portal.Background`, which writes the entry on the host for a Flatpak:
    /// the sandbox has an autostart folder of its own, which no desktop reads.
    BackgroundPortal,
}

impl Autostart {
    pub fn for_install(kind: InstallKind) -> Self {
        match kind {
            InstallKind::Flatpak => Self::BackgroundPortal,
            _ => Self::Entry,
        }
    }
}

/// The autostart entry of the executable `executable`, in the folder `config_home` stands for.
/// Every failure to read or write it reads as [`StartupRegistrationState::Unavailable`], as on
/// Windows.
#[derive(Debug)]
pub struct XdgAutostart {
    file: Option<PathBuf>,
    /// `None`: no path a desktop file can name, and nothing is ever written.
    executable: Option<String>,
}

impl XdgAutostart {
    /// The entry under `config_home`, the user's configuration folder, that starts `executable`.
    pub fn new(config_home: Option<&Path>, executable: Option<&Path>) -> Self {
        Self {
            file: config_home.map(|home| home.join("autostart").join(format!("{APP_ID}.desktop"))),
            executable: executable
                .filter(|path| crate::is_absolute(path))
                .and_then(Path::to_str)
                .map(str::to_owned),
        }
    }

    /// The entry of this process: under `$XDG_CONFIG_HOME`, for `$APPIMAGE` when an AppImage
    /// runs it and the executable otherwise.
    pub fn for_this_process() -> Self {
        let variable = |name: &str| std::env::var_os(name);
        let executable = installed_executable(variable("APPIMAGE"), std::env::current_exe().ok());
        Self::new(config_home(variable).as_deref(), executable.as_deref())
    }

    fn try_state(&self) -> io::Result<StartupRegistrationState> {
        let (Some(file), Some(executable)) = (&self.file, &self.executable) else {
            return Ok(StartupRegistrationState::Unavailable);
        };
        Ok(match read(file)? {
            None => StartupRegistrationState::Disabled,
            Some(entry) => entry.state(executable),
        })
    }

    fn try_set_enabled(&self, enabled: bool) -> io::Result<StartupRegistrationState> {
        let (Some(file), Some(executable)) = (&self.file, &self.executable) else {
            return Ok(StartupRegistrationState::Unavailable);
        };
        let entry = read(file)?;
        if enabled {
            if entry
                .is_none_or(|entry| entry.state(executable) != StartupRegistrationState::Enabled)
            {
                write(file, desktop_file(executable).as_bytes())?;
            }
        } else if entry.is_some_and(|entry| entry.starts(executable)) {
            std::fs::remove_file(file)?;
        }
        self.try_state()
    }
}

impl StartupPort for XdgAutostart {
    fn state(&self) -> StartupRegistrationState {
        self.try_state()
            .unwrap_or(StartupRegistrationState::Unavailable)
    }

    /// Enabling writes this install's entry unless it is already there, repairing one that is
    /// turned off or starts another path. Disabling removes only an entry that starts this
    /// install, and reports another path's as a mismatch.
    fn set_enabled(&self, enabled: bool) -> StartupRegistrationState {
        self.try_set_enabled(enabled)
            .unwrap_or(StartupRegistrationState::Unavailable)
    }

    /// Never: the Flatpak's portal writes its entry under the same file name, so this folder
    /// starts one copy at most, and an entry of the Flatpak reads as one for another path.
    fn other_copy_starts(&self) -> bool {
        false
    }
}

/// The user's configuration folder: `$XDG_CONFIG_HOME` when it is absolute, else `~/.config`.
fn config_home(variable: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    base_directory(variable, "XDG_CONFIG_HOME", ".config")
}

/// What the entry starts: the AppImage file itself, since its executable lives in a mount that
/// changes at every run, else the executable.
fn installed_executable(
    appimage: Option<OsString>,
    current_exe: Option<PathBuf>,
) -> Option<PathBuf> {
    appimage
        .map(PathBuf::from)
        .filter(|path| crate::is_absolute(path))
        .or(current_exe)
}

/// The entry that starts `executable` at sign-in, as Bientôt writes it.
fn desktop_file(executable: &str) -> String {
    let exec = exec_value(executable, AUTOSTART_ARGUMENT);
    format!(
        "[Desktop Entry]\nType=Application\nName=Bientôt\nExec={exec}\nIcon={APP_ID}\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

impl Entry {
    /// The state the table of the contract gives an autostart entry for this install's
    /// `executable`.
    fn state(&self, executable: &str) -> StartupRegistrationState {
        if self.turned_off {
            StartupRegistrationState::Disabled
        } else if self.starts(executable) {
            StartupRegistrationState::Enabled
        } else {
            StartupRegistrationState::RegistrationMismatch
        }
    }
}

#[cfg(test)]
mod tests;
