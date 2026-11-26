//! Start at sign-in outside a sandbox (research.md, R10; contracts/linux-backends.md, « Start at
//! sign-in »): an XDG autostart entry, `$XDG_CONFIG_HOME/autostart/dev.bientot.Bientot.desktop`,
//! whose `Exec` starts this install with [`AUTOSTART_ARGUMENT`]. The desktop starts every entry
//! of that folder as the user signs in, unless the entry is `Hidden` or
//! `X-GNOME-Autostart-enabled=false`.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use bientot_platform::startup::{AUTOSTART_ARGUMENT, StartupPort, StartupRegistrationState};

use crate::APP_ID;

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
                .filter(|path| path.is_absolute())
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
                write(file, &desktop_file(executable))?;
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

    /// No other install kind starts from this folder until the Flatpak and the AppImage have
    /// ids of their own: the portal names its entry after the same app id, which reads as an
    /// entry for another path.
    fn other_copy_starts(&self) -> bool {
        false
    }
}

/// The user's configuration folder: `$XDG_CONFIG_HOME` when it is absolute, else `~/.config`.
/// The base directory specification has a relative value ignored.
fn config_home(variable: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let absolute = |name| {
        variable(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    absolute("XDG_CONFIG_HOME").or_else(|| absolute("HOME").map(|home| home.join(".config")))
}

/// What the entry starts: the AppImage file itself, since its executable lives in a mount that
/// changes at every run, else the executable.
fn installed_executable(
    appimage: Option<OsString>,
    current_exe: Option<PathBuf>,
) -> Option<PathBuf> {
    appimage
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or(current_exe)
}

/// The entry that starts `executable` at sign-in, as Bientôt writes it.
fn desktop_file(executable: &str) -> String {
    let exec = escape_value(&format!("{} {AUTOSTART_ARGUMENT}", quoted(executable)));
    format!(
        "[Desktop Entry]\nType=Application\nName=Bientôt\nExec={exec}\nIcon={APP_ID}\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

/// What an autostart entry holds that tells its state.
#[derive(Debug, Default)]
struct Entry {
    /// `Hidden=true`, or `X-GNOME-Autostart-enabled=false`.
    turned_off: bool,
    /// The program and arguments of `Exec`; `None` without one, or when it cannot be read.
    exec: Option<Vec<String>>,
}

impl Entry {
    /// Reads the keys of the `[Desktop Entry]` group of `text`.
    fn parse(text: &str) -> Self {
        let mut entry = Self::default();
        let mut in_main_group = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_main_group = line == "[Desktop Entry]";
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if !in_main_group || line.starts_with('#') {
                continue;
            }
            let value = value.trim_start();
            match key.trim() {
                "Hidden" => entry.turned_off |= value == "true",
                "X-GNOME-Autostart-enabled" => entry.turned_off |= value == "false",
                "Exec" => entry.exec = unescape_value(value).as_deref().and_then(split_exec),
                _ => {}
            }
        }
        entry
    }

    /// The state the table of the contract gives the entry for this install's `executable`.
    fn state(&self, executable: &str) -> StartupRegistrationState {
        if self.turned_off {
            StartupRegistrationState::Disabled
        } else if self.starts(executable) {
            StartupRegistrationState::Enabled
        } else {
            StartupRegistrationState::RegistrationMismatch
        }
    }

    fn starts(&self, executable: &str) -> bool {
        self.exec
            .as_ref()
            .and_then(|exec| exec.first())
            .is_some_and(|program| program == executable)
    }
}

/// The entry in `file`, `None` when there is none.
fn read(file: &Path) -> io::Result<Option<Entry>> {
    match std::fs::read_to_string(file) {
        Ok(text) => Ok(Some(Entry::parse(&text))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Writes `text` to `file` through a file beside it, so that a desktop starting at that moment
/// reads the old entry or the new one, never half of one. The desktop reads only `*.desktop`.
fn write(file: &Path, text: &str) -> io::Result<()> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let mut partial = file.as_os_str().to_owned();
    partial.push(".partial");
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, file)
}

/// `argument` quoted as the `Exec` key's own rules have it, before the escapes of a string
/// value: in double quotes, with `"`, `` ` ``, `$` and `\` escaped by a backslash, and `%`
/// doubled, since a single one starts a field code.
fn quoted(argument: &str) -> String {
    let mut quoted = String::from('"');
    for character in argument.chars() {
        match character {
            '"' | '`' | '$' | '\\' => quoted.extend(['\\', character]),
            '%' => quoted.push_str("%%"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

/// The program and arguments of an `Exec` value, its string escapes already undone. Field codes
/// such as `%u` expand to nothing: an autostart entry has no file to open. `None` when a quote
/// is left open.
fn split_exec(exec: &str) -> Option<Vec<String>> {
    let mut arguments = Vec::new();
    let mut current: Option<String> = None;
    let mut quoted = false;
    let mut characters = exec.chars();
    while let Some(character) = characters.next() {
        match character {
            ' ' | '\t' if !quoted => arguments.extend(current.take()),
            '"' => {
                quoted = !quoted;
                current.get_or_insert_default();
            }
            '\\' => current.get_or_insert_default().extend(characters.next()),
            '%' => {
                if characters.next() == Some('%') {
                    current.get_or_insert_default().push('%');
                }
            }
            _ => current.get_or_insert_default().push(character),
        }
    }
    if quoted {
        return None;
    }
    arguments.extend(current);
    Some(arguments)
}

/// A string value as a desktop file holds it: `\` doubled, and the line breaks and tabs a value
/// cannot hold written as escapes.
fn escape_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\t' => escaped.push_str("\\t"),
            '\r' => escaped.push_str("\\r"),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// The escapes of a string value undone; `None` for one the specification does not define.
fn unescape_value(value: &str) -> Option<String> {
    let mut text = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            text.push(character);
            continue;
        }
        text.push(match characters.next()? {
            's' => ' ',
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            '\\' => '\\',
            _ => return None,
        });
    }
    Some(text)
}

#[cfg(test)]
mod tests;
