//! The system this process runs on, as « À propos » and the diagnostic export describe it: how
//! Bientôt was installed and the session from the environment, the distribution from os-release.

use std::ffi::OsString;
use std::path::Path;

use bientot_platform::system::{InstallKind, SessionType, SystemDescription, SystemInfo};

use crate::placement::desktops;

/// The system of this process's environment.
pub struct ThisSystem;

impl SystemInfo for ThisSystem {
    fn install_kind(&self) -> InstallKind {
        install_kind(
            |name| std::env::var_os(name),
            Path::new("/.flatpak-info").exists(),
        )
    }

    fn describe(&self) -> SystemDescription {
        let os_release = os_release_paths(self.install_kind())
            .iter()
            .find_map(|path| std::fs::read_to_string(path).ok());
        describe(|name| std::env::var_os(name), os_release.as_deref())
    }
}

/// A Flatpak when the sandbox's `/.flatpak-info` exists or `FLATPAK_ID` is set, else an AppImage
/// when `APPIMAGE` is, else a build run where it was made.
pub fn install_kind(
    variable: impl Fn(&str) -> Option<OsString>,
    flatpak_info: bool,
) -> InstallKind {
    if flatpak_info || is_set(&variable, "FLATPAK_ID") {
        InstallKind::Flatpak
    } else if is_set(&variable, "APPIMAGE") {
        InstallKind::AppImage
    } else {
        InstallKind::Development
    }
}

/// `WAYLAND_DISPLAY` set → Wayland; else `DISPLAY` set → X11; else none, as GPUI chooses.
pub fn session_type(variable: impl Fn(&str) -> Option<OsString>) -> Option<SessionType> {
    if is_set(&variable, "WAYLAND_DISPLAY") {
        Some(SessionType::Wayland)
    } else if is_set(&variable, "DISPLAY") {
        Some(SessionType::X11)
    } else {
        None
    }
}

/// Where to read os-release, first found first: in a Flatpak, the host's, which the sandbox
/// shows under `/run/host`, since its own `/etc` names the runtime; then `/etc`, then `/usr/lib`,
/// as os-release(5) says.
pub fn os_release_paths(kind: InstallKind) -> &'static [&'static str] {
    const SYSTEM: [&str; 2] = ["/etc/os-release", "/usr/lib/os-release"];
    match kind {
        InstallKind::Flatpak => &["/run/host/os-release", SYSTEM[0], SYSTEM[1]],
        _ => &SYSTEM,
    }
}

/// The system an environment that `variable` reads describes, with `os_release` the text of
/// os-release when one was read. Only `ID` and `VERSION_ID` are taken from it: `ID` is `linux`
/// when missing, as os-release(5) says.
pub fn describe(
    variable: impl Fn(&str) -> Option<OsString>,
    os_release: Option<&str>,
) -> SystemDescription {
    let os_release = os_release.unwrap_or_default();
    SystemDescription {
        distribution: os_release_value(os_release, "ID").unwrap_or_else(|| "linux".to_owned()),
        distribution_version: os_release_value(os_release, "VERSION_ID"),
        desktops: variable("XDG_CURRENT_DESKTOP")
            .and_then(|value| value.into_string().ok())
            .map(|value| desktops(&value))
            .unwrap_or_default(),
        session_type: session_type(variable),
    }
}

fn is_set(variable: &impl Fn(&str) -> Option<OsString>, name: &str) -> bool {
    variable(name).is_some_and(|value| !value.is_empty())
}

/// The last value os-release assigns to `key`, unquoted as the shell would; `None` when it
/// assigns none, or an empty one.
fn os_release_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| line.trim().split_once('='))
        .rfind(|(name, _)| *name == key)
        .map(|(_, value)| unquote(value.trim()))
        .filter(|value| !value.is_empty())
}

/// A value in single quotes as it is, one in double quotes with `\` taking the next character
/// literally, any other as it is.
fn unquote(value: &str) -> String {
    if let Some(inner) = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        return inner.to_owned();
    }
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        return value.to_owned();
    };
    let mut unquoted = String::with_capacity(inner.len());
    let mut characters = inner.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => unquoted.extend(characters.next()),
            _ => unquoted.push(character),
        }
    }
    unquoted
}

#[cfg(test)]
mod tests;
