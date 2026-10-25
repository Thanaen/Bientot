//! A migrated ledger in a temporary directory, the UTC calendar and the accounting the service
//! tests use.

use std::cell::Cell;

use bientot_domain::accounting::{
    ActivityCoverage, ActivitySegment, ActivityState, AutomaticOrigin,
};
use bientot_domain::configuration::Settings;
use bientot_domain::intervals::UtcInterval;
use bientot_domain::sources::SourceHealth;
use bientot_domain::time::{Clock, FixedTimeZoneProvider};
use bientot_store::activity::ActivityError;
use bientot_store::adjustments::ManualAdjustmentRepository;
use bientot_store::sessions::SessionEventRepository;
use jiff::Timestamp;
use jiff::civil::date;
use jiff::tz::TimeZone;
use rusqlite::Connection;
use tempfile::TempDir;

use crate::accounting::WorkdayAccounting;
use crate::sources::{ActivityReadResult, ActivitySource};

pub(crate) struct Database {
    directory: TempDir,
    pub(crate) connection: Connection,
}

impl Database {
    /// A ledger at the latest schema.
    pub(crate) fn new() -> Self {
        let database = Self::unmigrated();
        bientot_store::database::migrate(&database.connection).unwrap();
        database
    }

    /// A ledger with no tables, so that any read of it fails.
    pub(crate) fn unmigrated() -> Self {
        let directory = TempDir::new().unwrap();
        let connection =
            bientot_store::database::open(&directory.path().join("worktime.db")).unwrap();
        Self {
            directory,
            connection,
        }
    }

    /// Another connection to the same file, as a restarted app or a concurrent writer opens.
    pub(crate) fn reopen(&self) -> Connection {
        bientot_store::database::open(&self.directory.path().join("worktime.db")).unwrap()
    }
}

pub(crate) fn utc(year: i16, month: i8, day: i8, hour: i8) -> Timestamp {
    date(year, month, day)
        .at(hour, 0, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

pub(crate) fn utc_zones() -> FixedTimeZoneProvider {
    FixedTimeZoneProvider::new("UTC", TimeZone::UTC)
}

/// A frozen clock that counts its reads.
pub(crate) struct CountingClock {
    now: Timestamp,
    pub(crate) reads: Cell<usize>,
}

impl CountingClock {
    pub(crate) fn new(now: Timestamp) -> Self {
        Self {
            now,
            reads: Cell::new(0),
        }
    }
}

impl Clock for CountingClock {
    fn now(&self) -> Timestamp {
        self.reads.set(self.reads.get() + 1);
        self.now
    }
}

/// Active throughout every read, as a healthy source reports it.
pub(crate) struct FullyActive;

impl ActivitySource for FullyActive {
    fn read(&self, interval: UtcInterval) -> Result<ActivityReadResult, ActivityError> {
        Ok(ActivityReadResult::Snapshot {
            coverage: ActivityCoverage::new(
                interval,
                [ActivitySegment::new(
                    interval,
                    ActivityState::Active,
                    AutomaticOrigin::new("activitywatch:test").unwrap(),
                )],
            ),
            health: SourceHealth::healthy("activitywatch:test", interval.end()),
        })
    }
}

fn default_settings() -> Settings {
    Settings::default()
}

/// The accounting of a UTC machine with the default settings, reading `source` and the ledger
/// on `connection`.
pub(crate) fn utc_accounting<'a>(
    connection: &'a Connection,
    source: &'a dyn ActivitySource,
    clock: &'a dyn Clock,
    zones: &'a FixedTimeZoneProvider,
) -> WorkdayAccounting<'a> {
    WorkdayAccounting::new(
        source,
        SessionEventRepository::new(connection, clock),
        ManualAdjustmentRepository::new(connection),
        clock,
        zones,
        &default_settings,
    )
}

pub(crate) fn count(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

/// Makes every adjustment insert on `connection` fail, as a full disk would, until
/// [`restore_adjustment_inserts`].
pub(crate) fn fail_adjustment_inserts(connection: &Connection) {
    connection
        .execute_batch(
            "CREATE TEMP TRIGGER fail_adjustment_insert BEFORE INSERT ON ManualAdjustment
             BEGIN SELECT RAISE(ABORT, 'simulated transient storage failure'); END;",
        )
        .unwrap();
}

pub(crate) fn restore_adjustment_inserts(connection: &Connection) {
    connection
        .execute_batch("DROP TRIGGER fail_adjustment_insert;")
        .unwrap();
}
