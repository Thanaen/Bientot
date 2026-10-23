//! `settings.json`, ported from `Settings/JsonSettingsStore.cs`.

mod json;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use worktime_domain::configuration::{SettingsError, WorkTimeSettings};

use crate::file;

/// The line break .NET writes between indented lines: `Environment.NewLine`.
const NEWLINE: &str = if cfg!(windows) { "\r\n" } else { "\n" };

/// Reads and writes one `settings.json`.
#[derive(Clone, Debug)]
pub struct SettingsStore {
    path: PathBuf,
}

/// Settings read by [`SettingsStore::load`], and where they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedSettings {
    pub settings: WorkTimeSettings,
    pub source: SettingsSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsSource {
    /// Read from the file.
    File,
    /// No file yet, so the settings are the defaults: a first launch.
    Missing,
    /// The file could not be read or held invalid values. It was moved to `backup` and the
    /// settings are the defaults.
    Recovered { backup: PathBuf },
}

/// What the file holds, before [`SettingsStore::load`] moves an invalid one aside.
enum Read {
    Missing,
    Valid(Box<WorkTimeSettings>),
    Invalid,
}

#[derive(Debug)]
pub enum LoadSettingsError {
    /// The file was written by a newer WorkTime. It is left untouched.
    NewerSchema {
        schema_version: i32,
    },
    Io(io::Error),
}

impl std::fmt::Display for LoadSettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NewerSchema { schema_version } => write!(
                f,
                "these settings were saved by a newer WorkTime version (schema {schema_version})"
            ),
            Self::Io(error) => write!(f, "the settings could not be read: {error}"),
        }
    }
}

impl std::error::Error for LoadSettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NewerSchema { .. } => None,
            Self::Io(error) => Some(error),
        }
    }
}

#[derive(Debug)]
pub enum SaveSettingsError {
    /// Nothing was written.
    Invalid(Vec<SettingsError>),
    Io(io::Error),
}

impl std::fmt::Display for SaveSettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(errors) => {
                f.write_str("invalid workday settings: ")?;
                for (index, error) in errors.iter().enumerate() {
                    let separator = if index == 0 { "" } else { "; " };
                    write!(f, "{separator}{error}")?;
                }
                Ok(())
            }
            Self::Io(error) => write!(f, "the settings could not be written: {error}"),
        }
    }
}

impl std::error::Error for SaveSettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Invalid(_) => None,
            Self::Io(error) => Some(error),
        }
    }
}

impl SettingsStore {
    pub fn new(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            path: std::path::absolute(path)?,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the settings. A file that cannot be parsed, or holds invalid values, is moved aside
    /// to `settings.json.invalid-<32 hex digits>.json` and the defaults are returned. Schema 0
    /// reads as the current schema; a newer schema is an error.
    pub fn load(&self) -> Result<LoadedSettings, LoadSettingsError> {
        match self.read()? {
            Read::Missing => Ok(LoadedSettings {
                settings: WorkTimeSettings::default(),
                source: SettingsSource::Missing,
            }),
            Read::Valid(settings) => Ok(LoadedSettings {
                settings: *settings,
                source: SettingsSource::File,
            }),
            Read::Invalid => {
                let backup =
                    file::sibling(&self.path, &format!(".invalid-{}.json", file::random_hex()));
                fs::rename(&self.path, &backup).map_err(LoadSettingsError::Io)?;
                Ok(LoadedSettings {
                    settings: WorkTimeSettings::default(),
                    source: SettingsSource::Recovered { backup },
                })
            }
        }
    }

    /// The settings [`Self::load`] would read, without moving anything: the defaults when there
    /// is no file yet, nothing when the file does not read, holds invalid values or comes from a
    /// newer WorkTime.
    pub fn peek(&self) -> Option<WorkTimeSettings> {
        match self.read() {
            Ok(Read::Missing) => Some(WorkTimeSettings::default()),
            Ok(Read::Valid(settings)) => Some(*settings),
            Ok(Read::Invalid) | Err(_) => None,
        }
    }

    fn read(&self) -> Result<Read, LoadSettingsError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Read::Missing),
            Err(error) => return Err(LoadSettingsError::Io(error)),
        };
        let parsed = parse(&bytes);
        if let Ok(settings) = &parsed
            && settings.schema_version > WorkTimeSettings::CURRENT_SCHEMA_VERSION
        {
            return Err(LoadSettingsError::NewerSchema {
                schema_version: settings.schema_version,
            });
        }
        let settings = parsed.ok().map(|mut settings| {
            if settings.schema_version == 0 {
                settings.schema_version = WorkTimeSettings::CURRENT_SCHEMA_VERSION;
            }
            settings
        });
        Ok(match settings {
            Some(settings) if settings.validate().is_empty() => Read::Valid(Box::new(settings)),
            _ => Read::Invalid,
        })
    }

    /// Validates, then replaces the file atomically: the settings are written and flushed to
    /// `settings.json.<32 hex digits>.tmp`, which is then renamed over the file. The members of
    /// the replaced file these settings do not know are kept, which .NET does not do.
    pub fn save(&self, settings: &WorkTimeSettings) -> Result<(), SaveSettingsError> {
        let errors = settings.validate();
        if !errors.is_empty() {
            return Err(SaveSettingsError::Invalid(errors));
        }
        if let Some(directory) = self.path.parent() {
            fs::create_dir_all(directory).map_err(SaveSettingsError::Io)?;
        }
        let previous = fs::read(&self.path).ok();
        let previous = previous.as_deref().and_then(|bytes| {
            std::str::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)).ok()
        });
        let text = json::write(settings, NEWLINE, previous);
        file::replace(&self.path, text.as_bytes()).map_err(SaveSettingsError::Io)
    }
}

/// Why the bytes of a `settings.json` would not load as they are.
pub(crate) enum Unreadable {
    /// Saved by a newer WorkTime.
    Newer,
    /// Not settings, or invalid values that a load would set aside for the defaults.
    Invalid,
}

/// Whether `bytes` load as valid settings, as [`SettingsStore::load`] reads them.
pub(crate) fn check(bytes: &[u8]) -> Result<(), Unreadable> {
    let settings = parse(bytes).map_err(|_| Unreadable::Invalid)?;
    if settings.schema_version > WorkTimeSettings::CURRENT_SCHEMA_VERSION {
        return Err(Unreadable::Newer);
    }
    let mut settings = settings;
    if settings.schema_version == 0 {
        settings.schema_version = WorkTimeSettings::CURRENT_SCHEMA_VERSION;
    }
    if settings.validate().is_empty() {
        Ok(())
    } else {
        Err(Unreadable::Invalid)
    }
}

/// The settings in a `settings.json`, which may start with a UTF-8 byte order mark.
fn parse(bytes: &[u8]) -> Result<WorkTimeSettings, String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    std::str::from_utf8(bytes)
        .map_err(|error| error.to_string())
        .and_then(json::read)
}

#[cfg(test)]
mod tests;
