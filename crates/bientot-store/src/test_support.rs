//! A migrated ledger in a temporary directory, for the repository tests.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use tempfile::TempDir;

use crate::database::{LATEST_SCHEMA_VERSION, migrate, migrate_to, open};

pub(crate) struct TemporaryDatabase {
    directory: TempDir,
    pub(crate) connection: Connection,
}

impl TemporaryDatabase {
    /// A database at the latest schema.
    pub(crate) fn new() -> Self {
        Self::at_version(LATEST_SCHEMA_VERSION as usize)
    }

    pub(crate) fn at_version(version: usize) -> Self {
        let directory = TempDir::new().unwrap();
        let connection = open(&directory.path().join("worktime.db")).unwrap();
        assert_eq!(migrate_to(&connection, version).unwrap(), version as i64);
        Self {
            directory,
            connection,
        }
    }

    pub(crate) fn migrate(&self) -> i64 {
        migrate(&self.connection).unwrap()
    }

    pub(crate) fn path(&self) -> PathBuf {
        self.directory.path().join("worktime.db")
    }

    /// Another connection to the same file, as a restarted app or a concurrent writer opens.
    pub(crate) fn reopen(&self) -> Connection {
        open(&self.path()).unwrap()
    }

    pub(crate) fn count(&self, table: &str) -> i64 {
        count(&self.connection, table)
    }
}

pub(crate) fn count(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table};"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

pub(crate) fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}
