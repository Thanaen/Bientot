//! The SQLite ledger: connections, ported from `Database/SqliteConnectionFactory.cs`, and the
//! schema migrations, ported from `Database/MigrationRunner.cs`.

mod schema;

use std::io;
use std::path::Path;

use jiff::Timestamp;
use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::codec::format_instant;

/// The schema version [`migrate`] brings a database to.
pub const LATEST_SCHEMA_VERSION: i64 = schema::MIGRATIONS.len() as i64;

/// Opens or creates the database at `path`, creating its directory, with foreign keys enforced,
/// write-ahead logging, and up to 5 seconds of waiting for another writer.
///
/// Unlike the .NET factory, the connection does not use SQLite's shared cache: SQLite discourages
/// it, and without it a locked database waits for `busy_timeout` instead of failing at once.
pub fn open(path: &Path) -> Result<Connection, DatabaseError> {
    let path = std::path::absolute(path)?;
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let connection = Connection::open(&path)?;
    connection.execute_batch(
        "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;",
    )?;
    Ok(connection)
}

/// Applies every pending migration in one `BEGIN IMMEDIATE` transaction and returns the schema
/// version. A database whose schema is newer than this build is left untouched and refused.
pub fn migrate(connection: &Connection) -> Result<i64, DatabaseError> {
    run(connection, schema::MIGRATIONS, &Hooks::default())
}

/// Applies the migrations up to schema `version` only, for tests that start from an older ledger.
#[cfg(test)]
pub(crate) fn migrate_to(connection: &Connection, version: usize) -> Result<i64, DatabaseError> {
    run(
        connection,
        &schema::MIGRATIONS[..version],
        &Hooks::default(),
    )
}

#[derive(Debug)]
pub enum DatabaseError {
    Io(io::Error),
    Sqlite(rusqlite::Error),
    /// The database was migrated by a newer WorkTime.
    NewerSchema {
        found: i64,
        supported: i64,
    },
}

impl std::fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "the database could not be opened: {error}"),
            Self::Sqlite(error) => write!(f, "SQLite failed: {error}"),
            Self::NewerSchema { found, supported } => write!(
                f,
                "Database schema {found} is newer than the supported schema {supported}."
            ),
        }
    }
}

impl std::error::Error for DatabaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::NewerSchema { .. } => None,
        }
    }
}

impl From<io::Error> for DatabaseError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for DatabaseError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// One schema version: statements run in order, then recorded in `SchemaVersion` under `name`.
struct Migration {
    version: i64,
    name: &'static str,
    statements: &'static [&'static str],
    /// Compacts the file once the migration is committed, after one that frees many pages.
    vacuum_after_commit: bool,
}

/// Points where tests pause or interfere with a running migration.
#[derive(Default)]
struct Hooks<'a> {
    before_ownership: Option<&'a (dyn Fn() + Sync)>,
    after_version_read: Option<&'a (dyn Fn() + Sync)>,
    before_vacuum: Option<&'a (dyn Fn(&Connection) + Sync)>,
}

fn run(
    connection: &Connection,
    migrations: &[Migration],
    hooks: &Hooks,
) -> Result<i64, DatabaseError> {
    assert!(
        migrations
            .iter()
            .zip(1..)
            .all(|(migration, version)| migration.version == version
                && !migration.statements.is_empty()),
        "migrations must be numbered from 1 without gaps and have statements"
    );
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS SchemaVersion (
    Version INTEGER NOT NULL PRIMARY KEY,
    Name TEXT NOT NULL,
    AppliedAtUtc TEXT NOT NULL
);",
    )?;
    if let Some(hook) = hooks.before_ownership {
        hook();
    }

    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let mut version: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(Version), 0) FROM SchemaVersion;",
        [],
        |row| row.get(0),
    )?;
    if let Some(hook) = hooks.after_version_read {
        hook();
    }
    let supported = migrations.len() as i64;
    if version > supported {
        return Err(DatabaseError::NewerSchema {
            found: version,
            supported,
        });
    }
    let mut vacuum = false;
    let pending = &migrations[usize::try_from(version).unwrap_or(0)..];
    for migration in pending {
        for statement in migration.statements {
            transaction.execute_batch(statement)?;
        }
        transaction.execute(
            "INSERT INTO SchemaVersion (Version, Name, AppliedAtUtc) VALUES (?1, ?2, ?3);",
            (
                migration.version,
                migration.name,
                format_instant(Timestamp::now()),
            ),
        )?;
        version = migration.version;
        vacuum |= migration.vacuum_after_commit;
    }
    transaction.commit()?;

    // Compaction only returns free pages to the file system. The migrated schema is already
    // committed, so a failed VACUUM must not keep the application from starting.
    if vacuum {
        if let Some(hook) = hooks.before_vacuum {
            hook(connection);
        }
        match connection.execute_batch("VACUUM;") {
            Ok(()) => {}
            Err(error) if reported_by_sqlite(&error) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(version)
}

/// An error SQLite reported, what Microsoft.Data.Sqlite raises as a `SqliteException`.
fn reported_by_sqlite(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(..) | rusqlite::Error::SqlInputError { .. }
    )
}

/// One unit of reads and writes on `connection`: a transaction of its own, or, inside one a
/// caller already opened, a savepoint of it, so that several units commit or roll back as one.
/// Dropped without [`Scope::commit`], it rolls back. A nested scope takes the locks of the
/// transaction around it: open that one [`TransactionBehavior::Immediate`] when a scope within
/// it writes. Scopes end in the order opposite to the one they began in, as their borrows do.
pub struct Scope<'c> {
    connection: &'c Connection,
    nested: bool,
    done: bool,
}

thread_local! {
    /// The connection of each scope open on this thread: a transaction open on a connection
    /// with none is not one to join.
    static OPEN: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub fn scope(
    connection: &Connection,
    behavior: TransactionBehavior,
) -> rusqlite::Result<Scope<'_>> {
    // A transaction left open by mistake fails to begin another, rather than taking its writes.
    let key = std::ptr::from_ref(connection) as usize;
    let within = OPEN.with_borrow(|open| open.contains(&key));
    // SQLite ended the transaction around this scope on an error: whatever follows is undone.
    if within && connection.is_autocommit() {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ABORT),
            Some("the transaction around this scope was rolled back".to_owned()),
        ));
    }
    let nested = within && !connection.is_autocommit();
    connection.execute_batch(match (nested, behavior) {
        (true, _) => "SAVEPOINT worktime_scope",
        (false, TransactionBehavior::Deferred) => "BEGIN DEFERRED",
        (false, TransactionBehavior::Immediate) => "BEGIN IMMEDIATE",
        (false, _) => "BEGIN EXCLUSIVE",
    })?;
    OPEN.with_borrow_mut(|open| open.push(key));
    Ok(Scope {
        connection,
        nested,
        done: false,
    })
}

impl Scope<'_> {
    pub fn commit(mut self) -> rusqlite::Result<()> {
        self.connection.execute_batch(if self.nested {
            "RELEASE worktime_scope"
        } else {
            "COMMIT"
        })?;
        self.done = true;
        Ok(())
    }
}

impl std::ops::Deref for Scope<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        self.connection
    }
}

impl Drop for Scope<'_> {
    fn drop(&mut self) {
        let key = std::ptr::from_ref(self.connection) as usize;
        OPEN.with_borrow_mut(|open| {
            if let Some(last) = open.iter().rposition(|open| *open == key) {
                open.remove(last);
            }
        });
        if self.done {
            return;
        }
        let rolled_back = self.connection.execute_batch(if self.nested {
            "ROLLBACK TO worktime_scope; RELEASE worktime_scope"
        } else {
            "ROLLBACK"
        });
        // A savepoint that did not roll back leaves its writes to the transaction around it,
        // which is rolled back instead: its commit then fails rather than keeping them.
        if rolled_back.is_err() && !self.connection.is_autocommit() {
            let _ = self.connection.execute_batch("ROLLBACK");
        }
    }
}

#[cfg(test)]
mod tests;
