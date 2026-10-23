//! `app-state.json`: what WorkTime remembers between launches that is not a setting. It holds
//! only the local date of the last usage signal the service recorded. The .NET app has no such
//! file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use jiff::civil::Date;
use serde_json::{Map, Value};

use crate::file;

const USAGE_PING_LAST_SENT: &str = "usagePingLastSent";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AppState {
    /// The local date of the last usage signal the service answered with `204`.
    pub usage_ping_last_sent: Option<Date>,
}

/// Reads and writes one `app-state.json`.
#[derive(Clone, Debug)]
pub struct AppStateStore {
    path: PathBuf,
}

impl AppStateStore {
    pub fn new(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            path: std::path::absolute(path)?,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The saved state. A missing file, or one that does not hold a `YYYY-MM-DD` date, reads as
    /// the default: the next signal then counts as the first of the month, and its save
    /// replaces the file.
    pub fn load(&self) -> io::Result<AppState> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(parse(&bytes).unwrap_or_default()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(AppState::default()),
            Err(error) => Err(error),
        }
    }

    /// Replaces the file atomically, the way `settings.json` is replaced.
    pub fn save(&self, state: &AppState) -> io::Result<()> {
        if let Some(directory) = self.path.parent() {
            fs::create_dir_all(directory)?;
        }
        let mut object = Map::new();
        object.insert(
            USAGE_PING_LAST_SENT.to_owned(),
            state
                .usage_ping_last_sent
                .map_or(Value::Null, |date| Value::String(date.to_string())),
        );
        let mut text = serde_json::to_string_pretty(&Value::Object(object))?;
        text.push('\n');
        file::replace(&self.path, text.as_bytes())
    }
}

fn parse(bytes: &[u8]) -> Option<AppState> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let usage_ping_last_sent = match value.get(USAGE_PING_LAST_SENT)? {
        Value::Null => None,
        Value::String(text) if text.len() == 10 => Some(text.parse().ok()?),
        _ => return None,
    };
    Some(AppState {
        usage_ping_last_sent,
    })
}

#[cfg(test)]
mod tests;
