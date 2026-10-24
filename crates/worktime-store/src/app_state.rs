//! `app-state.json`: what WorkTime remembers between launches that is not a setting: the local
//! date of the last usage signal the service recorded, and the flyout callouts the user closed
//! today. The .NET app has no such file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

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
    /// Whether the user closed the callout of the count `id` on `today`.
    pub fn has_count(&self, today: Date, id: &str) -> bool {
        self.date == Some(today) && self.counts.iter().any(|count| count == id)
    }

    /// Whether the user closed the offer of the appointment `id` on `today`.
    pub fn has_offer(&self, today: Date, id: &str) -> bool {
        self.date == Some(today) && self.offers.iter().any(|offer| offer == id)
    }

    /// Closes the callout of the count `id` on `today`, forgetting those of another day. False
    /// when it was already closed.
    pub fn close_count(&mut self, today: Date, id: &str) -> bool {
        self.start(today);
        insert(&mut self.counts, id)
    }

    /// Closes the offer of the appointment `id` on `today`, forgetting those of another day.
    /// False when it was already closed.
    pub fn close_offer(&mut self, today: Date, id: &str) -> bool {
        self.start(today);
        insert(&mut self.offers, id)
    }

    fn start(&mut self, today: Date) {
        if self.date != Some(today) {
            *self = Self {
                date: Some(today),
                ..Self::default()
            };
        }
    }
}

fn insert(ids: &mut Vec<String>, id: &str) -> bool {
    let absent = !ids.iter().any(|known| known == id);
    if absent {
        ids.push(id.to_owned());
    }
    absent
}

/// Reads and writes one `app-state.json`. Its clones share one lock, so that the usage signal
/// on its thread and the flyout never interleave their updates.
#[derive(Clone, Debug)]
pub struct AppStateStore {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl AppStateStore {
    pub fn new(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            path: std::path::absolute(path)?,
            lock: Arc::default(),
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
        let _held = self.hold();
        self.write(state)
    }

    /// Reads the saved state, changes it with `change` and saves it, all under the lock of the
    /// store, so that each writer keeps what the others saved.
    pub fn update(&self, change: impl FnOnce(&mut AppState)) -> io::Result<()> {
        let _held = self.hold();
        let mut state = self.load()?;
        change(&mut state);
        self.write(&state)
    }

    fn hold(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self, state: &AppState) -> io::Result<()> {
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
