//! `app-state.json`: what WorkTime remembers between launches that is not a setting: the local
//! date of the last usage signal the service recorded, and the flyout callouts the user closed
//! today. The .NET app has no such file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use jiff::civil::Date;
use serde_json::{Map, Value};

use crate::file;

const USAGE_PING_LAST_SENT: &str = "usagePingLastSent";
const CLOSED_CALLOUTS: &str = "closedCallouts";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppState {
    /// The local date of the last usage signal the service answered with `204`.
    pub usage_ping_last_sent: Option<Date>,
    pub closed_callouts: ClosedCallouts,
}

/// The callouts of the flyout the user closed on `date`: the automatic counts by their
/// adjustment id, and the offers to count automatically by their appointment id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClosedCallouts {
    pub date: Option<Date>,
    pub counts: Vec<String>,
    pub offers: Vec<String>,
}

impl ClosedCallouts {
    /// What the user closed on `today`, nothing when the callouts were closed another day.
    pub fn on(&self, today: Date) -> (&[String], &[String]) {
        if self.date == Some(today) {
            (&self.counts, &self.offers)
        } else {
            (&[], &[])
        }
    }
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

    /// The saved state. A missing file reads as the default, and so does each part that does
    /// not hold what it should: a date that is not `YYYY-MM-DD` makes the next signal count as
    /// the first of the month, and the next save replaces the file.
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
        let closed = &state.closed_callouts;
        if let Some(date) = closed.date {
            object.insert(
                CLOSED_CALLOUTS.to_owned(),
                serde_json::json!({
                    "date": date.to_string(),
                    "counts": closed.counts,
                    "offers": closed.offers,
                }),
            );
        }
        let mut text = serde_json::to_string_pretty(&Value::Object(object))?;
        text.push('\n');
        file::replace(&self.path, text.as_bytes())
    }

    /// Reads the saved state, changes it with `change` and saves it, so that each writer keeps
    /// what the others saved.
    pub fn update(&self, change: impl FnOnce(&mut AppState)) -> io::Result<()> {
        let mut state = self.load()?;
        change(&mut state);
        self.save(&state)
    }
}

fn parse(bytes: &[u8]) -> Option<AppState> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let object = value.as_object()?;
    Some(AppState {
        usage_ping_last_sent: object.get(USAGE_PING_LAST_SENT).and_then(date),
        closed_callouts: object
            .get(CLOSED_CALLOUTS)
            .and_then(closed_callouts)
            .unwrap_or_default(),
    })
}

fn date(value: &Value) -> Option<Date> {
    match value {
        Value::String(text) if text.len() == 10 => text.parse().ok(),
        _ => None,
    }
}

fn closed_callouts(value: &Value) -> Option<ClosedCallouts> {
    let ids = |key: &str| -> Option<Vec<String>> {
        value
            .get(key)?
            .as_array()?
            .iter()
            .map(|id| id.as_str().map(str::to_owned))
            .collect()
    };
    Some(ClosedCallouts {
        date: Some(date(value.get("date")?)?),
        counts: ids("counts")?,
        offers: ids("offers")?,
    })
}

#[cfg(test)]
mod tests;
