//! The transfer file that moves a data folder to another computer: a copy of the ledger with one
//! more table, `WorkTimeTransfer`, holding `settings.json` and when the copy was made. SQLite
//! checks the file as it opens it, and an older ledger in it migrates like any other.
//!
//! An import replaces the folder's ledger and settings: a ledger cannot be merged with another
//! one, whose summaries, credit and reviews were computed from its own sessions. It is staged in
//! the folder, then applied by the next start, before anything opens the ledger. The ledger and
//! settings it replaces are first exported to `backups`, so that importing that file undoes it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::codec::{format_instant, parse_date, parse_instant};
use crate::data_directory::DataDirectory;
use crate::database::LATEST_SCHEMA_VERSION;
use crate::file;

/// The extension of a transfer file, without its dot.
pub const EXTENSION: &str = "worktime";

/// The layout of `WorkTimeTransfer` this build writes and reads.
const FORMAT: i64 = 1;

const CREATE_TABLE: &str = "CREATE TABLE WorkTimeTransfer (
    Format INTEGER NOT NULL,
    ExportedAtUtc TEXT NOT NULL,
    Settings BLOB NULL
);";

/// What a transfer file holds, as the import asks to confirm it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contents {
    pub exported_at: Timestamp,
    /// The first and the last workday with a summary, `None` for a ledger without any.
    pub days: Option<(Date, Date)>,
}

/// An import the start applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub contents: Contents,
    /// The export of the ledger and settings it replaced, `None` when the folder had no ledger.
    pub backup: Option<PathBuf>,
}

#[derive(Debug)]
pub enum TransferError {
    /// Not a file this build exported: another SQLite file, another kind of file, or a damaged
    /// one.
    NotATransfer,
    /// Exported by a newer WorkTime, whose ledger or settings this build cannot read.
    NewerVersion,
    Io(io::Error),
    Sqlite(rusqlite::Error),
}

impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotATransfer => f.write_str("the file is not a WorkTime export"),
            Self::NewerVersion => f.write_str("the file was exported by a newer WorkTime"),
            Self::Io(error) => write!(f, "the transfer file could not be written: {error}"),
            Self::Sqlite(error) => write!(f, "SQLite failed: {error}"),
        }
    }
}

impl std::error::Error for TransferError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotATransfer | Self::NewerVersion => None,
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
        }
    }
}

impl From<io::Error> for TransferError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for TransferError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// Writes the ledger of `directory` and its `settings.json` to `destination`, replacing it once
/// complete. The ledger is read on a connection of its own, so the app's goes on meanwhile.
pub fn export(
    directory: &DataDirectory,
    destination: &Path,
    now: Timestamp,
) -> Result<Contents, TransferError> {
    let settings = match fs::read(directory.settings_file()) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let ledger = Connection::open_with_flags(
        directory.database_file(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let temporary = file::sibling(destination, &format!(".{}.tmp", file::random_hex()));
    let written = write(&ledger, settings.as_deref(), &temporary, now)
        .and_then(|()| Ok(fs::rename(&temporary, destination)?));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written?;
    // The settings are copied as they are: a hand-edited settings.json that no longer reads
    // still leaves an export, which the import then refuses.
    Ok(open_transfer(destination)?.0)
}

fn write(
    ledger: &Connection,
    settings: Option<&[u8]>,
    temporary: &Path,
    now: Timestamp,
) -> Result<(), TransferError> {
    let path = temporary.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the destination is not Unicode",
        )
    })?;
    ledger.execute("VACUUM INTO ?1;", [path])?;
    let copy = Connection::open(temporary)?;
    copy.execute_batch(CREATE_TABLE)?;
    copy.execute(
        "INSERT INTO WorkTimeTransfer (Format, ExportedAtUtc, Settings) VALUES (?1, ?2, ?3);",
        (FORMAT, format_instant(now), settings),
    )?;
    copy.close().map_err(|(_, error)| error)?;
    Ok(())
}

/// What `file` holds, once it read as a transfer this build can import.
pub fn inspect(file: &Path) -> Result<Contents, TransferError> {
    Ok(read(file)?.0)
}

/// Reads `file` read-only: its contents and the settings it carries, once they read.
fn read(file: &Path) -> Result<(Contents, Option<Vec<u8>>), TransferError> {
    let (contents, settings) = open_transfer(file)?;
    if let Some(settings) = &settings {
        crate::settings::check(settings).map_err(|error| match error {
            crate::settings::Unreadable::Newer => TransferError::NewerVersion,
            crate::settings::Unreadable::Invalid => TransferError::NotATransfer,
        })?;
    }
    Ok((contents, settings))
}

/// Reads `file` read-only as a transfer: its contents and the settings it carries, unchecked.
fn open_transfer(file: &Path) -> Result<(Contents, Option<Vec<u8>>), TransferError> {
    let connection = Connection::open_with_flags(
        file,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(not_a_transfer)?;
    let check: String = connection
        .query_row("PRAGMA quick_check;", [], |row| row.get(0))
        .map_err(not_a_transfer)?;
    if check != "ok" {
        return Err(TransferError::NotATransfer);
    }
    let row: Option<(i64, String, Option<Vec<u8>>)> = connection
        .query_row(
            "SELECT Format, ExportedAtUtc, Settings FROM WorkTimeTransfer;",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(not_a_transfer)?;
    let Some((format, exported_at, settings)) = row else {
        return Err(TransferError::NotATransfer);
    };
    if format > FORMAT {
        return Err(TransferError::NewerVersion);
    }
    let exported_at = parse_instant(&exported_at).map_err(|_| TransferError::NotATransfer)?;
    let schema: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(Version), 0) FROM SchemaVersion;",
            [],
            |row| row.get(0),
        )
        .map_err(not_a_transfer)?;
    if schema > LATEST_SCHEMA_VERSION {
        return Err(TransferError::NewerVersion);
    }
    let days = workdays(&connection)?;
    Ok((Contents { exported_at, days }, settings))
}

/// A table missing, or a file SQLite does not read, means another kind of file.
fn not_a_transfer(error: rusqlite::Error) -> TransferError {
    match error {
        rusqlite::Error::SqliteFailure(..)
        | rusqlite::Error::SqlInputError { .. }
        | rusqlite::Error::InvalidColumnType(..) => TransferError::NotATransfer,
        error => TransferError::Sqlite(error),
    }
}

/// The first and the last workday with a summary. A schema 1 ledger already has the table.
fn workdays(connection: &Connection) -> Result<Option<(Date, Date)>, TransferError> {
    let (first, last): (Option<String>, Option<String>) = connection
        .query_row(
            "SELECT MIN(WorkdayDate), MAX(WorkdayDate) FROM DailySummaryRevision;",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(not_a_transfer)?;
    let date = |text: Option<String>| -> Result<Option<Date>, TransferError> {
        text.map(|text| parse_date(&text).map_err(|_| TransferError::NotATransfer))
            .transpose()
    };
    Ok(date(first)?.zip(date(last)?))
}

/// Copies `file`, once it read as a transfer, into `directory` for the next start to apply.
pub fn stage(file: &Path, directory: &DataDirectory) -> Result<Contents, TransferError> {
    let contents = inspect(file)?;
    let pending = directory.pending_import_file();
    // A PC where WorkTime never ran has no data folder yet.
    if let Some(parent) = pending.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = file::sibling(&pending, &format!(".{}.tmp", file::random_hex()));
    let copied = fs::copy(file, &temporary)
        .and_then(|_| file::sync(&temporary))
        .and_then(|()| fs::rename(&temporary, &pending));
    if copied.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    copied?;
    Ok(contents)
}

/// Applies the import [`stage`] left in `directory`, if any: exports the ledger and settings in
/// place to `backups`, then replaces them. Call it before anything opens the ledger. A staged
/// file that does not read is moved aside, so that the next start does not try it again; one
/// that failed for another reason stays for the next start.
pub fn apply_staged(
    directory: &DataDirectory,
    now: Timestamp,
    zone: &TimeZone,
) -> Result<Option<Applied>, TransferError> {
    let pending = directory.pending_import_file();
    if !pending.exists() {
        return Ok(None);
    }
    let (contents, settings) = match read(&pending) {
        Ok(read) => read,
        Err(error @ (TransferError::NotATransfer | TransferError::NewerVersion)) => {
            let aside = file::sibling(&pending, &format!(".invalid-{}", file::random_hex()));
            fs::rename(&pending, aside)?;
            return Err(error);
        }
        Err(error) => return Err(error),
    };
    let ledger = directory.database_file();
    let backup = if ledger.exists() {
        settle(&ledger)?;
        let backups = directory.backups_directory();
        fs::create_dir_all(&backups)?;
        let backup = backups.join(backup_name(now, zone));
        export(directory, &backup, now)?;
        Some(backup)
    } else {
        None
    };
    // Once the data is replaced, a start must not apply the file again over what was recorded
    // since: from here it no longer has the staged name, and a failure puts that name back.
    let applying = file::sibling(&pending, ".applying");
    fs::rename(&pending, &applying)?;
    let temporary = file::sibling(&ledger, &format!(".{}.tmp", file::random_hex()));
    let replaced = prepare(&applying, &temporary)
        .and_then(|()| replace_both(directory, settings.as_deref(), &temporary));
    if let Err(error) = replaced {
        let _ = fs::remove_file(&temporary);
        let _ = fs::rename(&applying, &pending);
        return Err(error);
    }
    // Left behind, it is only a copy of the file the user chose.
    let _ = fs::remove_file(&applying);
    Ok(Some(Applied { contents, backup }))
}

/// Writes the imported `settings`, then moves the ledger at `temporary` into place: both change,
/// or neither does. The ledger's move is the last step, so a failure before it puts the previous
/// settings back.
fn replace_both(
    directory: &DataDirectory,
    settings: Option<&[u8]>,
    temporary: &Path,
) -> Result<(), TransferError> {
    let path = directory.settings_file();
    let previous = match settings {
        Some(settings) => {
            let previous = match fs::read(&path) {
                Ok(previous) => Some(previous),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            file::replace(&path, settings)?;
            Some(previous)
        }
        None => None,
    };
    let moved = fs::rename(temporary, directory.database_file());
    if moved.is_err() {
        match previous {
            Some(Some(previous)) => file::replace(&path, &previous)?,
            Some(None) => fs::remove_file(&path)?,
            None => {}
        }
    }
    Ok(moved?)
}

/// Leaves the ledger at `path` whole in its own file: a write-ahead log beside it, which a crash
/// may leave, is replayed into it and removed, so that none is replayed into the imported ledger.
fn settle(path: &Path) -> Result<(), TransferError> {
    let connection = Connection::open(path)?;
    connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode = DELETE;")?;
    connection.close().map_err(|(_, error)| error)?;
    Ok(())
}

/// The ledger of the transfer at `pending`, without its transfer table, at `temporary`.
fn prepare(pending: &Path, temporary: &Path) -> Result<(), TransferError> {
    fs::copy(pending, temporary)?;
    let connection = Connection::open(temporary)?;
    connection.execute_batch("DROP TABLE WorkTimeTransfer;")?;
    connection.close().map_err(|(_, error)| error)?;
    file::sync(temporary)?;
    Ok(())
}

const BACKUP_PREFIX: &str = "before-import-";

/// `before-import-<local date>-<hours>-<minutes>-<seconds>.worktime`, on the clock the user
/// sees: sorts by time, and Windows accepts it as a file name.
fn backup_name(now: Timestamp, zone: &TimeZone) -> String {
    format!(
        "{BACKUP_PREFIX}{}.{EXTENSION}",
        now.to_zoned(zone.clone()).strftime("%Y-%m-%d-%H-%M-%S")
    )
}

/// The newest copy an import saved in `directory` before it replaced the data, if any.
pub fn latest_backup(directory: &DataDirectory) -> Option<PathBuf> {
    let suffix = format!(".{EXTENSION}");
    fs::read_dir(directory.backups_directory())
        .ok()?
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            (name.starts_with(BACKUP_PREFIX) && name.ends_with(&suffix)).then_some(name)
        })
        .max()
        .map(|name| directory.backups_directory().join(name))
}

/// The first and the last workday with a summary in the ledger of `directory`, read-only:
/// what the import would replace. `None` without a ledger or summaries.
pub fn ledger_days(directory: &DataDirectory) -> Result<Option<(Date, Date)>, TransferError> {
    let ledger = directory.database_file();
    if !ledger.exists() {
        return Ok(None);
    }
    let connection = Connection::open_with_flags(
        ledger,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    workdays(&connection)
}

#[cfg(test)]
mod tests;
