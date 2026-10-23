use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Barrier, Condvar, Mutex};
use std::time::Duration;

use rusqlite::types::Value;
use rusqlite::{named_params, params};
use tempfile::TempDir;

use super::schema::MIGRATIONS;
use super::*;
use crate::codec::parse_instant;

const TICKS_PER_MINUTE: i64 = 600_000_000;

struct TemporaryDatabase {
    directory: TempDir,
    connection: Connection,
}

impl TemporaryDatabase {
    fn new() -> Self {
        let directory = TempDir::new().unwrap();
        let connection = open(&directory.path().join("worktime.db")).unwrap();
        Self {
            directory,
            connection,
        }
    }

    /// A database at schema `version`, migrated by this port.
    fn at_version(version: usize) -> Self {
        let database = Self::new();
        assert_eq!(
            migrate_with(&database.connection, &MIGRATIONS[..version]).unwrap(),
            version as i64
        );
        database
    }

    /// A copy of a database the .NET generator wrote in `tests/fixtures`.
    fn copy_of_fixture(name: &str) -> Self {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("worktime.db");
        std::fs::copy(fixture_path(name), &path).unwrap();
        let connection = open(&path).unwrap();
        Self {
            directory,
            connection,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }
}

fn fixture_path(name: &str) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect()
}

fn migrate_with(connection: &Connection, migrations: &[Migration]) -> Result<i64, DatabaseError> {
    run(connection, migrations, &Hooks::default())
}

fn scalar(connection: &Connection, sql: &str) -> Value {
    connection.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn int(connection: &Connection, sql: &str) -> i64 {
    connection.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn text(connection: &Connection, sql: &str) -> String {
    connection.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn table_exists(connection: &Connection, table: &str) -> bool {
    connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1;",
            [table],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
        == 1
}

fn index_exists(connection: &Connection, index: &str) -> bool {
    connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1;",
            [index],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
        == 1
}

fn column_exists(connection: &Connection, table: &str, column: &str) -> bool {
    connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2;",
            [table, column],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
        == 1
}

fn is_sqlite_failure(result: Result<i64, DatabaseError>) -> bool {
    matches!(result, Err(DatabaseError::Sqlite(error)) if reported_by_sqlite(&error))
}

fn insert_adjustment(
    connection: &Connection,
    id: &str,
    supersedes_id: Option<&str>,
) -> rusqlite::Result<usize> {
    connection.execute(
        "INSERT INTO ManualAdjustment (
    Id,
    WorkdayDate,
    Kind,
    StartUtc,
    EndUtc,
    CreatedAtUtc,
    SupersedesId,
    TimezoneId)
VALUES (
    ?1,
    '2026-08-30',
    'add-work',
    '2026-08-30T07:00:00.0000000+00:00',
    '2026-08-30T08:00:00.0000000+00:00',
    '2026-08-30T08:00:01.0000000+00:00',
    ?2,
    'Romance Standard Time');",
        params![id, supersedes_id],
    )
}

fn insert_legacy_credit(connection: &Connection, minutes: Value) {
    connection
        .execute(
            "INSERT INTO CreditLedgerEntry (
    Id,
    WorkdayDate,
    WeekStartDate,
    Kind,
    Minutes,
    OriginWorkdayDate,
    OriginRevision,
    CreatedAtUtc)
VALUES (
    ' legacy-credit ',
    '2026-09-09',
    '2026-09-07',
    'applied',
    ?1,
    '2026-09-07',
    42,
    '2026-09-09T15:30:00.0000000+00:00');",
            [minutes],
        )
        .unwrap();
}

/// Copies a database the way `SqliteConnection.BackupDatabase` does in the .NET tests.
fn back_up(connection: &Connection, target: &std::path::Path) {
    connection
        .execute("VACUUM INTO ?1;", [target.to_str().unwrap()])
        .unwrap();
}

#[test]
fn latest_schema_is_applied_to_fresh_database() {
    let database = TemporaryDatabase::new();
    let connection = &database.connection;

    let version = migrate(connection).unwrap();

    assert_eq!(version, 11);
    assert!(column_exists(
        connection,
        "NotificationReceipt",
        "ReservedAtUtc"
    ));
    assert!(column_exists(connection, "SessionEvent", "OperationKey"));
    assert!(column_exists(connection, "ManualAdjustment", "TimezoneId"));
    assert!(column_exists(
        connection,
        "ManualAdjustment",
        "ConfirmsReviewId"
    ));
    assert!(table_exists(connection, "LongLockResolution"));
    assert!(column_exists(
        connection,
        "LongLockResolution",
        "CanonicalCreatedAtUtc"
    ));
    assert!(index_exists(connection, "UX_SessionEvent_OperationKey"));
    assert!(index_exists(connection, "UX_ManualAdjustment_SupersedesId"));
    assert!(column_exists(
        connection,
        "CreditLedgerEntry",
        "DurationTicks"
    ));
    assert!(!column_exists(connection, "CreditLedgerEntry", "Minutes"));
    assert!(index_exists(connection, "IX_CreditLedgerEntry_Week"));
    assert!(column_exists(
        connection,
        "DailySummaryRevision",
        "CreditedTicks"
    ));
    assert!(column_exists(
        connection,
        "DailySummaryRevision",
        "InputFingerprint"
    ));
    assert!(!column_exists(
        connection,
        "DailySummaryRevision",
        "CreditedMinutes"
    ));
    assert!(!column_exists(
        connection,
        "DailySummaryRevision",
        "SourceFingerprint"
    ));
    assert!(column_exists(
        connection,
        "DailySummaryRevision",
        "CarryDebtTicks"
    ));
    assert!(column_exists(
        connection,
        "DailySummaryRevision",
        "DebtPending"
    ));
    assert!(table_exists(connection, "ActivitySpan"));
    assert!(table_exists(connection, "ActivitySourceSelection"));
    assert_eq!(int(connection, "SELECT COUNT(*) FROM SchemaVersion;"), 11);
}

#[test]
fn schema_v1_database_upgrades_to_latest_without_changing_existing_rows() {
    let database = TemporaryDatabase::at_version(1);
    let connection = &database.connection;
    connection
        .execute_batch(
            "INSERT INTO SessionEvent (
    OccurredAtUtc,
    TimezoneId,
    Kind,
    Source,
    CreatedAtUtc)
VALUES (
    '2026-08-30T07:00:00.0000000+00:00',
    'Romance Standard Time',
    'locked',
    'windows-session',
    '2026-08-30T07:00:01.0000000+00:00');

INSERT INTO ManualAdjustment (
    Id,
    WorkdayDate,
    Kind,
    StartUtc,
    EndUtc,
    CreatedAtUtc)
VALUES (
    'legacy-adjustment',
    '2026-08-30',
    'add-work',
    '2026-08-30T07:00:00.0000000+00:00',
    '2026-08-30T08:00:00.0000000+00:00',
    '2026-08-30T08:00:01.0000000+00:00');

INSERT INTO NotificationReceipt (
    WorkdayDate, Trigger, EligibleAtUtc, DeliveredAtUtc, CombinedDeliveryId)
VALUES (
    '2026-08-30', 'completion-approach',
    '2026-08-30T16:00:00.0000000+00:00',
    '2026-08-30T16:01:00.0000000+00:00', 'legacy-delivery');",
        )
        .unwrap();

    let version = migrate(connection).unwrap();

    assert_eq!(version, 11);
    assert_eq!(int(connection, "SELECT COUNT(*) FROM SessionEvent;"), 1);
    assert_eq!(int(connection, "SELECT COUNT(*) FROM ManualAdjustment;"), 1);
    assert_eq!(
        int(
            connection,
            "SELECT COUNT(*) FROM SessionEvent WHERE OperationKey IS NULL;"
        ),
        1
    );
    assert_eq!(
        int(
            connection,
            "SELECT COUNT(*) FROM ManualAdjustment WHERE TimezoneId IS NULL;"
        ),
        1
    );
    assert_eq!(
        int(
            connection,
            "SELECT COUNT(*) FROM ManualAdjustment WHERE ConfirmsReviewId IS NULL;"
        ),
        1
    );
    assert_eq!(
        int(
            connection,
            "SELECT COUNT(*) FROM NotificationReceipt WHERE ReservedAtUtc = '2026-08-30T16:01:00.0000000+00:00';"
        ),
        1
    );
}

#[test]
fn schema_v2_database_upgrades_to_latest_without_changing_existing_rows() {
    let database = TemporaryDatabase::at_version(2);
    let connection = &database.connection;
    insert_adjustment(connection, "existing-adjustment", None).unwrap();

    let version = migrate(connection).unwrap();

    assert_eq!(version, 11);
    assert!(column_exists(
        connection,
        "ManualAdjustment",
        "ConfirmsReviewId"
    ));
    assert!(table_exists(connection, "LongLockResolution"));
    assert_eq!(
        int(
            connection,
            "SELECT COUNT(*) FROM ManualAdjustment WHERE ConfirmsReviewId IS NULL;"
        ),
        1
    );
}

#[test]
fn schema_v2_allows_roots_but_only_one_successor_per_adjustment() {
    let database = TemporaryDatabase::new();
    let connection = &database.connection;
    migrate(connection).unwrap();
    insert_adjustment(connection, "root-a", None).unwrap();
    insert_adjustment(connection, "root-b", None).unwrap();
    insert_adjustment(connection, "successor-a", Some("root-a")).unwrap();

    let second_successor = insert_adjustment(connection, "successor-b", Some("root-a"));

    assert!(
        matches!(second_successor, Err(rusqlite::Error::SqliteFailure(..))),
        "{second_successor:?}"
    );
    assert_eq!(int(connection, "SELECT COUNT(*) FROM ManualAdjustment;"), 3);
}

#[test]
fn legacy_credit_minutes_migrate_to_exact_ticks_without_changing_provenance() {
    for minutes in [7, 15_372_286_728] {
        let database = TemporaryDatabase::at_version(5);
        let connection = &database.connection;
        insert_legacy_credit(connection, Value::Integer(minutes));

        let version = migrate(connection).unwrap();
        let repeated_version = migrate(connection).unwrap();

        assert_eq!(version, 11);
        assert_eq!(repeated_version, 11);
        assert_eq!(
            text(connection, "SELECT Id FROM CreditLedgerEntry;"),
            " legacy-credit "
        );
        assert_eq!(
            text(connection, "SELECT WorkdayDate FROM CreditLedgerEntry;"),
            "2026-09-09"
        );
        assert_eq!(
            text(connection, "SELECT WeekStartDate FROM CreditLedgerEntry;"),
            "2026-09-07"
        );
        assert_eq!(
            text(connection, "SELECT Kind FROM CreditLedgerEntry;"),
            "applied"
        );
        assert_eq!(
            int(connection, "SELECT DurationTicks FROM CreditLedgerEntry;"),
            minutes * TICKS_PER_MINUTE
        );
        assert_eq!(
            text(
                connection,
                "SELECT OriginWorkdayDate FROM CreditLedgerEntry;"
            ),
            "2026-09-07"
        );
        assert_eq!(
            int(connection, "SELECT OriginRevision FROM CreditLedgerEntry;"),
            42
        );
        assert_eq!(
            text(connection, "SELECT CreatedAtUtc FROM CreditLedgerEntry;"),
            "2026-09-09T15:30:00.0000000+00:00"
        );
        assert_eq!(
            int(connection, "SELECT COUNT(*) FROM CreditLedgerEntry;"),
            1
        );
        assert!(index_exists(connection, "IX_CreditLedgerEntry_Week"));
    }
}

#[test]
fn invalid_legacy_credit_amount_rolls_back_schema_data_and_version() {
    for (invalid_amount, stored_type) in [
        (Value::Real(1.5), "real"),
        (Value::Integer(15_372_286_729), "integer"),
    ] {
        let database = TemporaryDatabase::at_version(5);
        let connection = &database.connection;
        insert_legacy_credit(connection, invalid_amount.clone());

        assert!(is_sqlite_failure(migrate(connection)));

        assert_eq!(
            int(connection, "SELECT MAX(Version) FROM SchemaVersion;"),
            5
        );
        assert!(column_exists(connection, "CreditLedgerEntry", "Minutes"));
        assert!(!column_exists(
            connection,
            "CreditLedgerEntry",
            "DurationTicks"
        ));
        assert!(!table_exists(connection, "CreditLedgerEntryV6"));
        assert_eq!(
            int(connection, "SELECT COUNT(*) FROM CreditLedgerEntry;"),
            1
        );
        assert_eq!(
            scalar(connection, "SELECT Minutes FROM CreditLedgerEntry;"),
            invalid_amount
        );
        assert_eq!(
            text(connection, "SELECT typeof(Minutes) FROM CreditLedgerEntry;"),
            stored_type
        );
        assert!(index_exists(connection, "IX_CreditLedgerEntry_Week"));
    }
}

#[test]
fn backup_can_be_restored_and_upgraded_before_destructive_credit_migration() {
    let source = TemporaryDatabase::at_version(5);
    insert_legacy_credit(&source.connection, Value::Integer(11));
    let backup_path = source.path("backup.db");
    back_up(&source.connection, &backup_path);

    migrate(&source.connection).unwrap();
    assert_eq!(
        int(
            &source.connection,
            "SELECT DurationTicks FROM CreditLedgerEntry;"
        ),
        11 * TICKS_PER_MINUTE
    );

    let backup = open(&backup_path).unwrap();
    let recovered_path = source.path("recovered.db");
    back_up(&backup, &recovered_path);
    let recovered = open(&recovered_path).unwrap();
    assert_eq!(
        int(&recovered, "SELECT MAX(Version) FROM SchemaVersion;"),
        5
    );
    migrate(&recovered).unwrap();
    assert_eq!(
        int(&recovered, "SELECT DurationTicks FROM CreditLedgerEntry;"),
        11 * TICKS_PER_MINUTE
    );
    assert_eq!(
        text(&recovered, "SELECT Id FROM CreditLedgerEntry;"),
        " legacy-credit "
    );
}

#[test]
fn schema_v1_is_applied_once_and_is_idempotent() {
    let database = TemporaryDatabase::new();
    let connection = &database.connection;

    let first_version = migrate_with(connection, &MIGRATIONS[..1]).unwrap();
    let second_version = migrate_with(connection, &MIGRATIONS[..1]).unwrap();

    assert_eq!(first_version, 1);
    assert_eq!(second_version, 1);
    assert!(table_exists(connection, "SessionEvent"));
    assert!(table_exists(connection, "DailySummaryRevision"));
    assert_eq!(int(connection, "SELECT COUNT(*) FROM SchemaVersion;"), 1);
}

#[test]
fn failed_migration_rolls_back_statements_and_version() {
    let database = TemporaryDatabase::new();
    let connection = &database.connection;
    let migrations = [
        Migration {
            version: 1,
            name: "stable",
            statements: &["CREATE TABLE Stable (Id INTEGER PRIMARY KEY);"],
            vacuum_after_commit: false,
        },
        Migration {
            version: 2,
            name: "broken",
            statements: &[
                "CREATE TABLE MustRollback (Id INTEGER PRIMARY KEY);",
                "THIS IS NOT VALID SQL;",
            ],
            vacuum_after_commit: false,
        },
    ];

    assert!(is_sqlite_failure(migrate_with(connection, &migrations)));

    assert!(!table_exists(connection, "Stable"));
    assert!(!table_exists(connection, "MustRollback"));
    assert_eq!(int(connection, "SELECT COUNT(*) FROM SchemaVersion;"), 0);
}

#[test]
fn failed_compaction_keeps_the_committed_migration() {
    let database = TemporaryDatabase::new();
    let connection = &database.connection;
    let migrations = [Migration {
        version: 1,
        name: "compacted",
        statements: &["CREATE TABLE Compacted (Id INTEGER PRIMARY KEY);"],
        vacuum_after_commit: true,
    }];
    // The .NET test keeps a reader open; an open transaction makes VACUUM fail the same way.
    let block_vacuum = |connection: &Connection| {
        connection.execute_batch("BEGIN;").unwrap();
        assert!(connection.execute_batch("VACUUM;").is_err());
    };
    let hooks = Hooks {
        before_vacuum: Some(&block_vacuum),
        ..Hooks::default()
    };

    let version = run(connection, &migrations, &hooks).unwrap();
    connection.execute_batch("ROLLBACK;").unwrap();

    assert_eq!(version, 1);
    assert!(table_exists(connection, "Compacted"));
    assert_eq!(int(connection, "SELECT COUNT(*) FROM SchemaVersion;"), 1);
}

/// Opens once and stays open, for one thread to wait on another.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    opened: Condvar,
}

impl Gate {
    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.opened.notify_all();
    }

    fn wait(&self, timeout: Duration) -> bool {
        let open = self.open.lock().unwrap();
        *self
            .opened
            .wait_timeout_while(open, timeout, |open| !*open)
            .unwrap()
            .0
    }
}

#[test]
fn concurrent_runners_converge_on_the_latest_schema() {
    let database = TemporaryDatabase::new();
    let connections = [(); 2].map(|()| open(&database.path("worktime.db")).unwrap());
    let both_at_ownership_boundary = Barrier::new(2);
    let first_version_read = Gate::default();
    let release_first_owner = Gate::default();
    let version_reads = AtomicUsize::new(0);
    let before_ownership = || {
        both_at_ownership_boundary.wait();
    };
    let after_version_read = || {
        if version_reads.fetch_add(1, Ordering::SeqCst) == 0 {
            first_version_read.open();
            assert!(release_first_owner.wait(Duration::from_secs(10)));
        }
    };
    let hooks = Hooks {
        before_ownership: Some(&before_ownership),
        after_version_read: Some(&after_version_read),
        ..Hooks::default()
    };

    let versions = std::thread::scope(|scope| {
        let hooks = &hooks;
        let runners = connections.map(|connection| {
            scope.spawn(move || run(&connection, &MIGRATIONS[..1], hooks).unwrap())
        });
        let first_read = first_version_read.wait(Duration::from_secs(10));
        let reads_while_owned = version_reads.load(Ordering::SeqCst);
        release_first_owner.open();
        assert!(first_read);
        assert_eq!(reads_while_owned, 1);
        runners.map(|runner| runner.join().unwrap())
    });

    assert_eq!(versions, [1, 1]);
    assert_eq!(version_reads.load(Ordering::SeqCst), 2);
    assert_eq!(
        int(&database.connection, "SELECT COUNT(*) FROM SchemaVersion;"),
        1
    );
    assert!(table_exists(&database.connection, "SessionEvent"));
}

const LEGACY_JSON: &str = " {\"legacy-detail\": true} \n";
const DURATION_NAMES: [&str; 9] = [
    "Credited",
    "Excluded",
    "Unknown",
    "NominalTarget",
    "EffectiveTarget",
    "CarryAvailable",
    "CarryApplied",
    "CarryEarned",
    "CarryExpired",
];

fn insert_legacy_summary(connection: &Connection, minutes: i64, workday_date: &str, revision: i64) {
    connection
        .execute(
            "INSERT INTO DailySummaryRevision VALUES (
    $workdayDate, $revision, '2026-09-07T12:00:00.1234567+00:00',
    'Romance Standard Time', ' legacy-input ',
    $minutes, $minutes, $minutes, $target, $target,
    $minutes, $minutes, $minutes, $minutes,
    '2026-09-07T17:00:00.1234567+00:00', 'green', 'complete', 1, $json);",
            named_params! {
                "$workdayDate": workday_date,
                "$revision": revision,
                "$minutes": minutes,
                "$target": minutes.max(1),
                "$json": LEGACY_JSON,
            },
        )
        .unwrap();
}

#[test]
fn all_duration_columns_convert_exactly_and_provenance_survives() {
    for minutes in [0, 15_372_286_728] {
        let database = TemporaryDatabase::at_version(6);
        let connection = &database.connection;
        insert_legacy_summary(connection, minutes, "2026-09-07", 1);

        assert_eq!(migrate(connection).unwrap(), 11);
        assert_eq!(migrate(connection).unwrap(), 11);

        for name in DURATION_NAMES {
            let expected = if minutes == 0 && name.ends_with("Target") {
                TICKS_PER_MINUTE
            } else {
                minutes * TICKS_PER_MINUTE
            };
            let sql = format!("SELECT {name}Ticks FROM DailySummaryRevision;");
            assert_eq!(int(connection, &sql), expected, "{name}");
        }
        let row = |column: &str| {
            scalar(
                connection,
                &format!("SELECT {column} FROM DailySummaryRevision;"),
            )
        };
        let text_value = |value: &str| Value::Text(value.to_owned());
        assert_eq!(row("WorkdayDate"), text_value("2026-09-07"));
        assert_eq!(row("Revision"), Value::Integer(1));
        assert_eq!(
            row("ComputedAtUtc"),
            text_value("2026-09-07T12:00:00.1234567+00:00")
        );
        assert_eq!(row("TimezoneId"), text_value("Romance Standard Time"));
        assert_eq!(row("InputFingerprint"), text_value(" legacy-input "));
        assert_eq!(
            row("ProjectedDepartureUtc"),
            text_value("2026-09-07T17:00:00.1234567+00:00")
        );
        assert_eq!(row("IndicatorState"), text_value("green"));
        assert_eq!(row("SourceQuality"), text_value("complete"));
        assert_eq!(row("HasPendingReview"), Value::Integer(1));
        assert_eq!(row("BreakdownJson"), text_value(LEGACY_JSON));
        assert_eq!(row("COUNT(*)"), Value::Integer(1));
    }
}

#[test]
fn invalid_duration_rolls_back_entire_rebuild() {
    for name in DURATION_NAMES {
        for invalid_value in [Value::Integer(15_372_286_729), Value::Real(1.5)] {
            let database = TemporaryDatabase::at_version(6);
            let connection = &database.connection;
            insert_legacy_summary(connection, 1, "2026-09-07", 1);
            connection
                .execute(
                    &format!("UPDATE DailySummaryRevision SET {name}Minutes = ?1;"),
                    [&invalid_value],
                )
                .unwrap();

            assert!(
                is_sqlite_failure(migrate(connection)),
                "{name} {invalid_value:?}"
            );

            assert_eq!(
                int(connection, "SELECT MAX(Version) FROM SchemaVersion;"),
                6
            );
            assert_eq!(
                scalar(
                    connection,
                    &format!("SELECT {name}Minutes FROM DailySummaryRevision;")
                ),
                invalid_value
            );
            assert_eq!(
                text(
                    connection,
                    "SELECT BreakdownJson FROM DailySummaryRevision;"
                ),
                LEGACY_JSON
            );
            assert_eq!(
                int(
                    connection,
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = 'DailySummaryRevisionV7';"
                ),
                0
            );
            assert_eq!(
                int(connection, "SELECT COUNT(*) FROM DailySummaryRevision;"),
                1
            );
            assert!(!column_exists(
                connection,
                "DailySummaryRevision",
                "CreditedTicks"
            ));
        }
    }
}

#[test]
fn legacy_backup_restores_after_original_upgrade_and_upgrades_again() {
    let original = TemporaryDatabase::at_version(6);
    insert_legacy_summary(&original.connection, 3, "2026-09-07", 1);
    let backup_path = original.path("backup.db");
    back_up(&original.connection, &backup_path);

    migrate(&original.connection).unwrap();
    let recovered_path = original.path("recovered.db");
    back_up(&open(&backup_path).unwrap(), &recovered_path);
    let recovered = open(&recovered_path).unwrap();

    assert_eq!(
        int(&recovered, "SELECT MAX(Version) FROM SchemaVersion;"),
        6
    );
    assert_eq!(
        int(
            &recovered,
            "SELECT CreditedMinutes FROM DailySummaryRevision;"
        ),
        3
    );
    assert_eq!(
        text(
            &recovered,
            "SELECT BreakdownJson FROM DailySummaryRevision;"
        ),
        LEGACY_JSON
    );
    migrate(&recovered).unwrap();
    assert_eq!(
        int(
            &recovered,
            "SELECT CreditedTicks FROM DailySummaryRevision;"
        ),
        1_800_000_000
    );
    assert_eq!(
        text(
            &recovered,
            "SELECT BreakdownJson FROM DailySummaryRevision;"
        ),
        LEGACY_JSON
    );
}

#[test]
fn unversioned_legacy_detail_is_preserved_and_can_be_superseded() {
    use jiff::SignedDuration;
    use jiff::civil::date;
    use worktime_domain::history::{
        DailySummaryBreakdown, DailySummaryRevision, DailySummarySnapshot,
        DailySummarySnapshotParts,
    };
    use worktime_domain::sources::SourceQuality;
    use worktime_domain::summaries::IndicatorState;
    use worktime_domain::time::LocalWorkday;

    use crate::history::{DailySummaryError, DailySummaryRepository};

    let database = TemporaryDatabase::at_version(6);
    insert_legacy_summary(&database.connection, 1, "2026-09-07", 1);
    migrate(&database.connection).unwrap();
    let repository = DailySummaryRepository::new(&database.connection);
    let day = date(2026, 9, 7);
    let undecodable = |result: Result<_, DailySummaryError>| {
        matches!(
            result,
            Err(DailySummaryError::Storage(
                crate::StorageError::InvalidStoredValue(_)
            ))
        )
    };

    assert!(undecodable(repository.read_latest(day).map(|_| ())));
    assert!(undecodable(repository.read_latest_week(day).map(|_| ())));

    let utc = |hour| parse_instant(&format!("2026-09-07T{hour:02}:00:00.0000000+00:00")).unwrap();
    let replacement = DailySummaryRevision::new(
        2,
        DailySummarySnapshot::new(DailySummarySnapshotParts {
            workday: LocalWorkday::new(day, "Romance Standard Time"),
            computed_at: utc(13),
            input_fingerprint: "recomputed-input".to_owned(),
            credited_work: SignedDuration::ZERO,
            excluded_work: SignedDuration::ZERO,
            unknown_work: SignedDuration::ZERO,
            nominal_target: SignedDuration::from_hours(7),
            effective_target: SignedDuration::from_hours(7),
            carry_available: SignedDuration::ZERO,
            carry_applied: SignedDuration::ZERO,
            carry_earned: SignedDuration::ZERO,
            carry_expired: SignedDuration::ZERO,
            carry_debt: SignedDuration::ZERO,
            debt_pending: false,
            projected_departure: Some(utc(20)),
            indicator_state: IndicatorState::Green,
            source_quality: SourceQuality::Complete,
            has_pending_review: false,
            breakdown: DailySummaryBreakdown::default(),
        })
        .unwrap(),
    )
    .unwrap();
    repository.append(&replacement).unwrap();

    assert_eq!(
        repository
            .read_latest(day)
            .unwrap()
            .map(|latest| latest.revision()),
        Some(2)
    );
    assert_eq!(repository.read_latest_week(day).unwrap().len(), 1);
    assert_eq!(
        int(
            &database.connection,
            "SELECT COUNT(*) FROM DailySummaryRevision;"
        ),
        1
    );
}

#[test]
fn upgrade_keeps_only_the_latest_revision_of_each_day() {
    let database = TemporaryDatabase::at_version(6);
    let connection = &database.connection;
    for revision in 1..=3 {
        insert_legacy_summary(connection, revision, "2026-09-07", revision);
    }
    insert_legacy_summary(connection, 5, "2026-09-08", 1);

    assert_eq!(migrate(connection).unwrap(), 11);

    assert_eq!(
        int(connection, "SELECT COUNT(*) FROM DailySummaryRevision;"),
        2
    );
    assert_eq!(
        int(
            connection,
            "SELECT Revision FROM DailySummaryRevision WHERE WorkdayDate = '2026-09-07';"
        ),
        3
    );
    assert_eq!(
        int(
            connection,
            "SELECT CreditedTicks FROM DailySummaryRevision WHERE WorkdayDate = '2026-09-07';"
        ),
        1_800_000_000
    );
    assert_eq!(
        int(
            connection,
            "SELECT Revision FROM DailySummaryRevision WHERE WorkdayDate = '2026-09-08';"
        ),
        1
    );
    assert_eq!(
        int(
            connection,
            "SELECT SUM(CarryDebtTicks) FROM DailySummaryRevision;"
        ),
        0
    );
    assert!(
        connection
            .execute("UPDATE DailySummaryRevision SET CarryDebtTicks = -1;", [])
            .is_err()
    );
    assert_eq!(
        int(
            connection,
            "SELECT SUM(DebtPending) FROM DailySummaryRevision;"
        ),
        0
    );
    assert!(
        connection
            .execute("UPDATE DailySummaryRevision SET DebtPending = 2;", [])
            .is_err()
    );
}

#[test]
fn opening_creates_the_directory_and_sets_the_connection_pragmas() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("WorkTime").join("worktime.db");

    let connection = open(&path).unwrap();

    assert!(path.exists());
    assert_eq!(text(&connection, "PRAGMA journal_mode;"), "wal");
    assert_eq!(int(&connection, "PRAGMA foreign_keys;"), 1);
    assert_eq!(int(&connection, "PRAGMA busy_timeout;"), 5000);
}

#[test]
fn a_newer_schema_is_refused_and_left_untouched() {
    let database = TemporaryDatabase::new();
    let connection = &database.connection;
    migrate(connection).unwrap();
    connection
        .execute_batch(
            "INSERT INTO SchemaVersion VALUES (12, 'From the future', '2030-01-01T00:00:00.0000000+00:00');",
        )
        .unwrap();

    let error = migrate(connection).unwrap_err();

    assert_eq!(
        error.to_string(),
        "Database schema 12 is newer than the supported schema 11."
    );
    assert_eq!(int(connection, "SELECT COUNT(*) FROM SchemaVersion;"), 12);
}

/// `tests/fixtures/migrations.json` lists the statements of `WorkTimeSchema.Migrations` as .NET
/// builds them, after interpolating the constants of V6 and V7.
#[test]
fn migrations_match_the_dotnet_schema() {
    let dotnet: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../tests/fixtures/migrations.json")).unwrap();
    assert_eq!(MIGRATIONS.len(), dotnet.len());
    for (migration, expected) in MIGRATIONS.iter().zip(&dotnet) {
        assert_eq!(migration.version, expected["version"].as_i64().unwrap());
        assert_eq!(migration.name, expected["name"].as_str().unwrap());
        assert_eq!(
            migration.vacuum_after_commit,
            expected["vacuumAfterCommit"].as_bool().unwrap()
        );
        let statements: Vec<_> = expected["statements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|statement| statement.as_str().unwrap())
            .collect();
        assert_eq!(
            migration.statements, statements,
            "migration {}",
            migration.version
        );
    }
}

/// Every schema object with its SQL text, and every row of every table. `SchemaVersion` keeps
/// its versions and names; when each migration ran differs between two databases.
fn dump(connection: &Connection) -> Vec<String> {
    let mut lines = Vec::new();
    let mut schema = connection
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name;")
        .unwrap();
    let objects = schema
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for (kind, name, table, sql) in objects {
        lines.push(format!("{kind} {name} on {table}: {sql:?}"));
        if kind != "table" {
            continue;
        }
        let sql = if name == "SchemaVersion" {
            "SELECT Version, Name FROM SchemaVersion ORDER BY Version;".to_owned()
        } else {
            format!("SELECT * FROM \"{name}\" ORDER BY rowid;")
        };
        let mut rows = connection.prepare(&sql).unwrap();
        let columns = rows.column_count();
        let values = rows
            .query_map([], |row| {
                (0..columns)
                    .map(|column| row.get::<_, Value>(column))
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap();
        for row in values {
            lines.push(format!("  {name}: {:?}", row.unwrap()));
        }
    }
    lines
}

fn applied_versions(connection: &Connection) -> Vec<(i64, String, String)> {
    connection
        .prepare("SELECT Version, Name, AppliedAtUtc FROM SchemaVersion ORDER BY Version;")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn a_database_migrated_by_dotnet_is_current() {
    let database = TemporaryDatabase::copy_of_fixture("schema-v11.db");
    let connection = &database.connection;
    let before = applied_versions(connection);

    let version = migrate(connection).unwrap();

    assert_eq!(version, LATEST_SCHEMA_VERSION);
    assert_eq!(applied_versions(connection), before);
    assert_eq!(before.len(), 11);
    for (_, _, applied_at) in &before {
        assert!(parse_instant(applied_at).is_ok(), "{applied_at}");
    }
}

#[test]
fn a_fresh_database_has_the_schema_dotnet_creates() {
    let dotnet = TemporaryDatabase::copy_of_fixture("schema-v11.db");
    let rust = TemporaryDatabase::new();

    migrate(&rust.connection).unwrap();

    assert_eq!(dump(&rust.connection), dump(&dotnet.connection));
    for (_, _, applied_at) in applied_versions(&rust.connection) {
        assert!(parse_instant(&applied_at).is_ok(), "{applied_at}");
    }
}

#[test]
fn a_legacy_dotnet_database_upgrades_as_dotnet_upgrades_it() {
    let legacy = TemporaryDatabase::copy_of_fixture("legacy-v1.db");
    let upgraded_by_dotnet = TemporaryDatabase::copy_of_fixture("legacy-v1-upgraded.db");

    assert_eq!(migrate(&legacy.connection).unwrap(), 11);

    let dump = dump(&legacy.connection);
    assert_eq!(dump, self::dump(&upgraded_by_dotnet.connection));
    assert!(dump.len() > 30, "{dump:#?}");
}

#[test]
fn a_scope_inside_another_commits_or_rolls_back_with_it() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("CREATE TABLE T (V INTEGER);")
        .unwrap();
    let rows = |connection: &Connection| -> i64 {
        connection
            .query_row("SELECT COUNT(*) FROM T;", [], |row| row.get(0))
            .unwrap()
    };

    // An inner scope dropped undoes only its own writes.
    let outer = scope(&connection, TransactionBehavior::Immediate).unwrap();
    outer.execute("INSERT INTO T VALUES (1);", []).unwrap();
    {
        let inner = scope(&outer, TransactionBehavior::Immediate).unwrap();
        inner.execute("INSERT INTO T VALUES (2);", []).unwrap();
    }
    assert_eq!(rows(&outer), 1);
    // An inner scope committed is undone with the outer one.
    let inner = scope(&outer, TransactionBehavior::Immediate).unwrap();
    inner.execute("INSERT INTO T VALUES (3);", []).unwrap();
    inner.commit().unwrap();
    drop(outer);
    assert_eq!(rows(&connection), 0);
    assert!(connection.is_autocommit());

    let outer = scope(&connection, TransactionBehavior::Immediate).unwrap();
    let inner = scope(&outer, TransactionBehavior::Deferred).unwrap();
    inner.execute("INSERT INTO T VALUES (4);", []).unwrap();
    inner.commit().unwrap();
    outer.commit().unwrap();
    assert_eq!(rows(&connection), 1);
}

#[test]
fn an_inner_scope_that_cannot_roll_back_fails_the_transaction_around_it() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("CREATE TABLE T (V INTEGER);")
        .unwrap();
    let outer = scope(&connection, TransactionBehavior::Immediate).unwrap();
    outer.execute("INSERT INTO T VALUES (1);", []).unwrap();
    {
        let inner = scope(&outer, TransactionBehavior::Immediate).unwrap();
        inner.execute("INSERT INTO T VALUES (2);", []).unwrap();
        // Its savepoint gone, the inner scope's rollback fails.
        inner.execute_batch("RELEASE worktime_scope;").unwrap();
    }

    assert!(outer.commit().is_err());
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM T;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
    assert!(connection.is_autocommit());
}

#[test]
fn a_scope_does_not_join_a_transaction_it_did_not_open() {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("BEGIN;").unwrap();

    assert!(scope(&connection, TransactionBehavior::Immediate).is_err());
    // Nor one open on another connection while a scope is open on its own.
    let other = Connection::open_in_memory().unwrap();
    let _open = scope(&other, TransactionBehavior::Immediate).unwrap();
    assert!(scope(&connection, TransactionBehavior::Immediate).is_err());
}

#[test]
fn a_scope_inside_a_transaction_sqlite_rolled_back_writes_nothing() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("CREATE TABLE T (V INTEGER);")
        .unwrap();
    let outer = scope(&connection, TransactionBehavior::Immediate).unwrap();
    outer.execute("INSERT INTO T VALUES (1);", []).unwrap();
    // As SQLite does on some errors: the transaction ends under the scope's feet.
    outer.execute_batch("ROLLBACK;").unwrap();

    assert!(scope(&outer, TransactionBehavior::Immediate).is_err());
    drop(outer);
    // Once the outer scope ended, a scope begins again.
    scope(&connection, TransactionBehavior::Immediate)
        .unwrap()
        .commit()
        .unwrap();
}
