//! The desktop entries Bientôt writes for itself, the autostart entry and the AppImage's launcher
//! entry: where they go, their `Exec` quoted and their values escaped as the Desktop Entry
//! Specification has them, and the keys read back from one.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// A base folder of the XDG specification: `$name` when it is absolute, else `under_home` in
/// `$HOME`. The specification has a relative value ignored.
pub(crate) fn base_directory(
    variable: impl Fn(&str) -> Option<OsString>,
    name: &str,
    under_home: &str,
) -> Option<PathBuf> {
    let absolute = |name| {
        variable(name)
            .map(PathBuf::from)
            .filter(|path| crate::is_absolute(path))
    };
    absolute(name).or_else(|| absolute("HOME").map(|home| home.join(under_home)))
}

/// What an entry holds that tells whether it starts this install, and whether it is on.
#[derive(Debug, Default)]
pub(crate) struct Entry {
    /// `Hidden=true`, or `X-GNOME-Autostart-enabled=false`.
    pub(crate) turned_off: bool,
    /// The program and arguments of `Exec`; `None` without one, or when it cannot be read.
    pub(crate) exec: Option<Vec<String>>,
}

impl Entry {
    /// Reads the keys of the `[Desktop Entry]` group of `text`.
    pub(crate) fn parse(text: &str) -> Self {
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

    /// Whether its program is `executable`.
    pub(crate) fn starts(&self, executable: &str) -> bool {
        self.exec
            .as_ref()
            .and_then(|exec| exec.first())
            .is_some_and(|program| program == executable)
    }
}

/// The entry in `file`, `None` when there is none.
pub(crate) fn read(file: &Path) -> io::Result<Option<Entry>> {
    match std::fs::read_to_string(file) {
        Ok(text) => Ok(Some(Entry::parse(&text))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Writes `contents` to `file` through a file beside it, so that a desktop reading the folder at
/// that moment reads the old file or the new one, never half of one. The desktop reads only
/// `*.desktop` and the icon theme only `*.png`.
pub(crate) fn write(file: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let mut partial = file.as_os_str().to_owned();
    partial.push(".partial");
    std::fs::write(&partial, contents)?;
    std::fs::rename(&partial, file)
}

/// Removes `file`; one already gone counts as removed.
pub(crate) fn remove(file: &Path) -> io::Result<()> {
    match std::fs::remove_file(file) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// The value of `Exec` that starts `program` with `arguments`, as written in the file: the
/// program quoted by the key's own rules, then the escapes of a string value.
pub(crate) fn exec_value(program: &str, arguments: &str) -> String {
    let command = if arguments.is_empty() {
        quoted(program)
    } else {
        format!("{} {arguments}", quoted(program))
    };
    escape_value(&command)
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
/// such as `%u` expand to nothing: Bientôt's entries have no file to open. `None` when a quote
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
pub(crate) fn escape_value(value: &str) -> String {
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
