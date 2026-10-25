//! Reading ledger rows: every stored value is checked the way the .NET readers check it, and a
//! value they would reject surfaces as [`InvalidStoredValue`] instead of a panic.

use jiff::Timestamp;
use jiff::civil::Date;
use rusqlite::Row;
use rusqlite::types::ValueRef;

use crate::codec::{parse_date, parse_instant, parse_instant_lenient, parse_instant_loose};

/// A stored row the .NET readers would reject, what they raise as `InvalidDataException`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidStoredValue {
    pub table: &'static str,
    pub detail: String,
}

impl std::fmt::Display for InvalidStoredValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} contains an invalid persisted value: {}",
            self.table, self.detail
        )
    }
}

impl std::error::Error for InvalidStoredValue {}

/// What every ledger read and write can fail with, whichever repository runs it.
#[derive(Debug)]
pub enum StorageError {
    Sqlite(rusqlite::Error),
    InvalidStoredValue(InvalidStoredValue),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "SQLite failed: {error}"),
            Self::InvalidStoredValue(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::InvalidStoredValue(error) => Some(error),
        }
    }
}

/// `From` conversions into the `Storage` variant of a repository error.
macro_rules! storage_error_conversions {
    ($error:ty) => {
        impl From<rusqlite::Error> for $error {
            fn from(error: rusqlite::Error) -> Self {
                Self::Storage($crate::row::StorageError::Sqlite(error))
            }
        }

        impl From<$crate::row::InvalidStoredValue> for $error {
            fn from(error: $crate::row::InvalidStoredValue) -> Self {
                Self::Storage($crate::row::StorageError::InvalidStoredValue(error))
            }
        }
    };
}

pub(crate) use storage_error_conversions;

/// The columns of one row of `table`, read by name.
pub(crate) struct StoredRow<'r, 's> {
    row: &'r Row<'s>,
    table: &'static str,
}

impl<'r, 's> StoredRow<'r, 's> {
    pub(crate) fn new(row: &'r Row<'s>, table: &'static str) -> Self {
        Self { row, table }
    }

    pub(crate) fn invalid(&self, detail: impl Into<String>) -> InvalidStoredValue {
        InvalidStoredValue {
            table: self.table,
            detail: detail.into(),
        }
    }

    fn value(&self, column: &str) -> ValueRef<'r> {
        self.row
            .get_ref(column)
            .expect("the query selects every column it reads")
    }

    pub(crate) fn optional_text(
        &self,
        column: &str,
    ) -> Result<Option<&'r str>, InvalidStoredValue> {
        match self.value(column) {
            ValueRef::Null => Ok(None),
            ValueRef::Text(bytes) => std::str::from_utf8(bytes)
                .map(Some)
                .map_err(|_| self.invalid(format!("{column} is not UTF-8 text"))),
            _ => Err(self.invalid(format!("{column} is not text"))),
        }
    }

    pub(crate) fn text(&self, column: &str) -> Result<&'r str, InvalidStoredValue> {
        self.optional_text(column)?
            .ok_or_else(|| self.invalid(format!("{column} is null")))
    }

    pub(crate) fn optional_integer(&self, column: &str) -> Result<Option<i64>, InvalidStoredValue> {
        match self.value(column) {
            ValueRef::Null => Ok(None),
            ValueRef::Integer(value) => Ok(Some(value)),
            _ => Err(self.invalid(format!("{column} is not an integer"))),
        }
    }

    pub(crate) fn integer(&self, column: &str) -> Result<i64, InvalidStoredValue> {
        self.optional_integer(column)?
            .ok_or_else(|| self.invalid(format!("{column} is null")))
    }

    /// A time zone identifier, which must be neither blank nor padded.
    pub(crate) fn time_zone_id(&self, column: &str) -> Result<&'r str, InvalidStoredValue> {
        self.canonical_time_zone_id(column, self.text(column)?)
    }

    pub(crate) fn canonical_time_zone_id<'t>(
        &self,
        column: &str,
        value: &'t str,
    ) -> Result<&'t str, InvalidStoredValue> {
        if value.trim().is_empty() || value.trim() != value {
            return Err(self.invalid(format!("{column} is not canonical")));
        }
        Ok(value)
    }

    pub(crate) fn date(&self, column: &str) -> Result<Date, InvalidStoredValue> {
        parse_date(self.text(column)?).map_err(|error| self.invalid(format!("{column}: {error}")))
    }

    /// An instant in the .NET round-trip format with any offset, as most .NET readers accept.
    pub(crate) fn instant(&self, column: &str) -> Result<Timestamp, InvalidStoredValue> {
        self.parsed_instant(column, self.text(column)?)
    }

    pub(crate) fn optional_instant(
        &self,
        column: &str,
    ) -> Result<Option<Timestamp>, InvalidStoredValue> {
        self.optional_text(column)?
            .map(|text| self.parsed_instant(column, text))
            .transpose()
    }

    fn parsed_instant(&self, column: &str, text: &str) -> Result<Timestamp, InvalidStoredValue> {
        parse_instant_loose(text).map_err(|error| self.invalid(format!("{column}: {error}")))
    }

    /// An instant in any shape [`parse_instant_lenient`] reads, as the credit ledger accepts.
    pub(crate) fn lenient_instant(&self, column: &str) -> Result<Timestamp, InvalidStoredValue> {
        parse_instant_lenient(self.text(column)?)
            .map_err(|error| self.invalid(format!("{column}: {error}")))
    }

    /// An instant exactly as `format_instant` writes it, as `PersistedUtcInstantParser` demands.
    pub(crate) fn strict_instant(&self, column: &str) -> Result<Timestamp, InvalidStoredValue> {
        parse_instant(self.text(column)?)
            .map_err(|error| self.invalid(format!("{column}: {error}")))
    }
}

/// Collects rows parsed by a `query_map` closure that returns each parse result as its value.
pub(crate) fn collect<T, E>(
    rows: impl Iterator<Item = rusqlite::Result<Result<T, InvalidStoredValue>>>,
) -> Result<Vec<T>, E>
where
    E: From<rusqlite::Error> + From<InvalidStoredValue>,
{
    rows.map(|row| Ok(row??)).collect()
}
