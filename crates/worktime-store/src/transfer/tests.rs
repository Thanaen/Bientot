use std::fs;

use jiff::Timestamp;
use jiff::civil::date;
use jiff::tz::TimeZone;
use rusqlite::Connection;
use tempfile::TempDir;
use worktime_domain::configuration::WorkTimeSettings;

use super::*;
use crate::database::{migrate, migrate_to, open};
use crate::settings::SettingsStore;

const NOW: &str = "2026-10-22T09:30:00Z";

fn now() -> Timestamp {
    NOW.parse().unwrap()
}

/// A data folder with a migrated ledger holding a summary for each of `days`, and settings with
/// a daily target of `target_minutes`.
fn folder(days: &[&str], target_minutes: i64) -> (TempDir, DataDirectory) {
    let temporary = TempDir::new().unwrap();
    let directory = DataDirectory::from(temporary.path().join("WorkTime"));
    let connection = open(&directory.database_file()).unwrap();
    migrate(&connection).unwrap();
    for day in days {
        insert_summary(&connection, day);
    }
    let settings = WorkTimeSettings {
        nominal_daily_target: jiff::SignedDuration::from_mins(target_minutes),
        ..WorkTimeSettings::default()
    };
    SettingsStore::new(directory.settings_file())
        .unwrap()
        .save(&settings)
        .unwrap();
    (temporary, directory)
}

fn insert_summary(connection: &Connection, day: &str) {
    connection
        .execute(
            "INSERT INTO DailySummaryRevision (WorkdayDate, Revision, ComputedAtUtc, TimezoneId,
                InputFingerprint, CreditedTicks, ExcludedTicks, UnknownTicks, NominalTargetTicks,
                EffectiveTargetTicks, CarryAvailableTicks, CarryAppliedTicks, CarryEarnedTicks,
                CarryExpiredTicks, ProjectedDepartureUtc, IndicatorState, SourceQuality,
                HasPendingReview, BreakdownJson)
             VALUES (?1, 1, '2026-10-01T00:00:00.0000000Z', 'Europe/Paris', 'f', 0, 0, 0, 1, 1,
                0, 0, 0, 0, NULL, 'OnTrack', 'Complete', 0, '{}');",
            [day],
        )
        .unwrap();
}

fn summaries(directory: &DataDirectory) -> Vec<String> {
    let connection = open(&directory.database_file()).unwrap();
    let mut statement = connection
        .prepare("SELECT WorkdayDate FROM DailySummaryRevision ORDER BY WorkdayDate;")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn target(directory: &DataDirectory) -> i64 {
    SettingsStore::new(directory.settings_file())
        .unwrap()
        .load()
        .unwrap()
        .settings
        .nominal_daily_target
        .as_mins()
}

#[test]
fn an_export_holds_the_ledger_its_period_and_when_it_was_made() {
    let (_temporary, directory) = folder(&["2026-03-02", "2026-10-21", "2026-05-12"], 450);
    let destination = directory.path().join("export.worktime");

    let contents = export(&directory, &destination, now()).unwrap();

    assert_eq!(
        contents,
        Contents {
            exported_at: now(),
            days: Some((date(2026, 3, 2), date(2026, 10, 21))),
        }
    );
    assert_eq!(inspect(&destination).unwrap(), contents);
}

#[test]
fn an_export_reads_what_the_app_wrote_and_has_not_checkpointed() {
    let (_temporary, directory) = folder(&[], 420);
    // The app's connection stays open: its writes are still in the write-ahead log.
    let app = open(&directory.database_file()).unwrap();
    insert_summary(&app, "2026-10-20");
    let destination = directory.path().join("export.worktime");

    let contents = export(&directory, &destination, now()).unwrap();

    assert_eq!(
        contents.days,
        Some((date(2026, 10, 20), date(2026, 10, 20)))
    );
    drop(app);
}

#[test]
fn a_ledger_without_summaries_exports_without_a_period() {
    let (_temporary, directory) = folder(&[], 420);
    let destination = directory.path().join("export.worktime");

    assert_eq!(export(&directory, &destination, now()).unwrap().days, None);
}

#[test]
fn an_import_replaces_the_ledger_and_the_settings_after_saving_them() {
    let (_old, old_pc) = folder(&["2026-03-02", "2026-10-21"], 450);
    let file = old_pc.path().join("export.worktime");
    export(&old_pc, &file, now()).unwrap();
    let (_new, new_pc) = folder(&["2026-10-22"], 480);

    stage(&file, &new_pc).unwrap();
    assert!(new_pc.pending_import_file().exists());
    let applied = apply_staged(&new_pc, now(), &TimeZone::UTC)
        .unwrap()
        .unwrap();

    assert_eq!(summaries(&new_pc), ["2026-03-02", "2026-10-21"]);
    assert_eq!(target(&new_pc), 450);
    assert!(!new_pc.pending_import_file().exists());
    let backup = applied.backup.unwrap();
    assert_eq!(
        backup,
        new_pc
            .backups_directory()
            .join("before-import-2026-10-22-09-30-00.worktime")
    );
    assert_eq!(latest_backup(&new_pc), Some(backup.clone()));
    assert_eq!(
        inspect(&backup).unwrap().days,
        Some((date(2026, 10, 22), date(2026, 10, 22)))
    );
    assert_eq!(
        applied.contents.days,
        Some((date(2026, 3, 2), date(2026, 10, 21)))
    );
}

#[test]
fn an_import_that_cannot_write_the_settings_changes_nothing_and_waits() {
    let (_old, old_pc) = folder(&["2026-03-02", "2026-10-21"], 450);
    let file = old_pc.path().join("export.worktime");
    export(&old_pc, &file, now()).unwrap();
    let (_new, new_pc) = folder(&["2026-10-22"], 480);
    stage(&file, &new_pc).unwrap();
    // A folder in place of settings.json: nothing can be written there.
    fs::remove_file(new_pc.settings_file()).unwrap();
    fs::create_dir(new_pc.settings_file()).unwrap();

    assert!(apply_staged(&new_pc, now(), &TimeZone::UTC).is_err());

    assert_eq!(summaries(&new_pc), ["2026-10-22"]);
    assert!(new_pc.pending_import_file().exists());
}

#[test]
fn a_ledger_that_cannot_move_into_place_puts_the_settings_back() {
    let (_temporary, directory) = folder(&["2026-10-22"], 480);
    let before = fs::read(directory.settings_file()).unwrap();
    let missing = directory.path().join("missing.tmp");

    assert!(replace_both(&directory, Some(b"{}"), &missing).is_err());

    assert_eq!(fs::read(directory.settings_file()).unwrap(), before);
    assert_eq!(summaries(&directory), ["2026-10-22"]);
}

#[test]
fn importing_the_backup_undoes_the_import() {
    let (_old, old_pc) = folder(&["2026-03-02"], 450);
    let file = old_pc.path().join("export.worktime");
    export(&old_pc, &file, now()).unwrap();
    let (_new, new_pc) = folder(&["2026-10-22"], 480);
    stage(&file, &new_pc).unwrap();
    let backup = apply_staged(&new_pc, now(), &TimeZone::UTC)
        .unwrap()
        .unwrap()
        .backup
        .unwrap();

    stage(&backup, &new_pc).unwrap();
    apply_staged(
        &new_pc,
        "2026-10-22T09:31:00Z".parse().unwrap(),
        &TimeZone::UTC,
    )
    .unwrap();

    assert_eq!(summaries(&new_pc), ["2026-10-22"]);
    assert_eq!(target(&new_pc), 480);
}

#[test]
fn the_imported_ledger_no_longer_holds_the_transfer_table_and_migrates() {
    let (_old, old_pc) = folder(&[], 450);
    let file = old_pc.path().join("export.worktime");
    export(&old_pc, &file, now()).unwrap();
    let (_new, new_pc) = folder(&[], 480);
    stage(&file, &new_pc).unwrap();
    apply_staged(&new_pc, now(), &TimeZone::UTC).unwrap();

    let connection = open(&new_pc.database_file()).unwrap();
    let tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'WorkTimeTransfer';",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
    assert_eq!(migrate(&connection).unwrap(), LATEST_SCHEMA_VERSION);
}

#[test]
fn an_older_ledger_imports_and_migrates_on_open() {
    let temporary = TempDir::new().unwrap();
    let old_pc = DataDirectory::from(temporary.path().join("old"));
    let connection = open(&old_pc.database_file()).unwrap();
    migrate_to(&connection, 3).unwrap();
    drop(connection);
    let file = temporary.path().join("export.worktime");
    export(&old_pc, &file, now()).unwrap();
    let (_new, new_pc) = folder(&[], 480);

    stage(&file, &new_pc).unwrap();
    apply_staged(&new_pc, now(), &TimeZone::UTC).unwrap();

    let connection = open(&new_pc.database_file()).unwrap();
    assert_eq!(migrate(&connection).unwrap(), LATEST_SCHEMA_VERSION);
    // No settings in the old folder: the new one keeps its own.
    assert_eq!(target(&new_pc), 480);
}

#[test]
fn a_write_ahead_log_left_by_a_crash_is_not_replayed_into_the_imported_ledger() {
    let (_old, old_pc) = folder(&["2026-03-02"], 450);
    let file = old_pc.path().join("export.worktime");
    export(&old_pc, &file, now()).unwrap();
    let (_new, new_pc) = folder(&[], 480);
    // What a crash leaves: the ledger and its log, copied while the writer's connection is open.
    let (_crashed, crashed) = folder(&[], 480);
    let writer = open(&crashed.database_file()).unwrap();
    insert_summary(&writer, "2026-10-22");
    for suffix in ["", "-wal", "-shm"] {
        let name = format!("worktime.db{suffix}");
        fs::copy(crashed.path().join(&name), new_pc.path().join(&name)).unwrap();
    }
    drop(writer);
    stage(&file, &new_pc).unwrap();

    let applied = apply_staged(&new_pc, now(), &TimeZone::UTC)
        .unwrap()
        .unwrap();

    assert_eq!(summaries(&new_pc), ["2026-03-02"]);
    assert_eq!(
        inspect(&applied.backup.unwrap()).unwrap().days,
        Some((date(2026, 10, 22), date(2026, 10, 22)))
    );
}

#[test]
fn a_folder_without_a_ledger_imports_without_a_backup() {
    let (_old, old_pc) = folder(&["2026-03-02"], 450);
    let file = old_pc.path().join("export.worktime");
    export(&old_pc, &file, now()).unwrap();
    let temporary = TempDir::new().unwrap();
    let new_pc = DataDirectory::from(temporary.path().join("WorkTime"));

    stage(&file, &new_pc).unwrap();
    let applied = apply_staged(&new_pc, now(), &TimeZone::UTC)
        .unwrap()
        .unwrap();

    assert_eq!(applied.backup, None);
    assert_eq!(summaries(&new_pc), ["2026-03-02"]);
    assert_eq!(target(&new_pc), 450);
}

#[test]
fn nothing_staged_applies_nothing() {
    let (_temporary, directory) = folder(&["2026-10-22"], 480);

    assert_eq!(
        apply_staged(&directory, now(), &TimeZone::UTC).unwrap(),
        None
    );
    assert_eq!(summaries(&directory), ["2026-10-22"]);
}

#[test]
fn a_settings_file_edited_into_nonsense_still_exports_and_backs_up() {
    let (_old, old_pc) = folder(&["2026-10-21"], 450);
    let file = old_pc.path().join("export.worktime");
    export(&old_pc, &file, now()).unwrap();
    let (_new, new_pc) = folder(&["2026-10-22"], 480);
    fs::write(new_pc.settings_file(), "{ not json").unwrap();
    let saved = new_pc.path().join("saved.worktime");

    export(&new_pc, &saved, now()).unwrap();
    stage(&file, &new_pc).unwrap();
    let applied = apply_staged(&new_pc, now(), &TimeZone::UTC)
        .unwrap()
        .unwrap();

    assert!(matches!(inspect(&saved), Err(TransferError::NotATransfer)));
    assert!(applied.backup.unwrap().exists());
    assert_eq!(summaries(&new_pc), ["2026-10-21"]);
    assert_eq!(target(&new_pc), 450);
}

#[test]
fn files_that_are_not_exports_are_refused() {
    let (_temporary, directory) = folder(&["2026-10-22"], 480);
    let text = directory.path().join("notes.worktime");
    fs::write(&text, "not a database").unwrap();
    let empty = directory.path().join("empty.worktime");
    fs::write(&empty, "").unwrap();

    for file in [&text, &empty, &directory.database_file()] {
        assert!(
            matches!(inspect(file), Err(TransferError::NotATransfer)),
            "{}",
            file.display()
        );
        assert!(matches!(
            stage(file, &directory),
            Err(TransferError::NotATransfer)
        ));
    }
    assert!(!directory.pending_import_file().exists());
}

#[test]
fn exports_of_a_newer_worktime_are_refused() {
    let (_temporary, directory) = folder(&[], 480);
    let newer_format = directory.path().join("format.worktime");
    export(&directory, &newer_format, now()).unwrap();
    Connection::open(&newer_format)
        .unwrap()
        .execute("UPDATE WorkTimeTransfer SET Format = Format + 1;", [])
        .unwrap();
    let newer_schema = directory.path().join("schema.worktime");
    export(&directory, &newer_schema, now()).unwrap();
    Connection::open(&newer_schema)
        .unwrap()
        .execute(
            "INSERT INTO SchemaVersion (Version, Name, AppliedAtUtc) VALUES (?1, 'Future', 'x');",
            [LATEST_SCHEMA_VERSION + 1],
        )
        .unwrap();
    let newer_settings = directory.path().join("settings.worktime");
    export(&directory, &newer_settings, now()).unwrap();
    Connection::open(&newer_settings)
        .unwrap()
        .execute(
            "UPDATE WorkTimeTransfer SET Settings = CAST('{\"schemaVersion\": 99}' AS BLOB);",
            [],
        )
        .unwrap();

    for file in [&newer_format, &newer_schema, &newer_settings] {
        assert!(
            matches!(inspect(file), Err(TransferError::NewerVersion)),
            "{}",
            file.display()
        );
    }
}

#[test]
fn a_staged_file_that_no_longer_reads_is_moved_aside() {
    let (_temporary, directory) = folder(&["2026-10-22"], 480);
    fs::write(directory.pending_import_file(), "damaged").unwrap();

    assert!(matches!(
        apply_staged(&directory, now(), &TimeZone::UTC),
        Err(TransferError::NotATransfer)
    ));
    assert!(!directory.pending_import_file().exists());
    assert_eq!(summaries(&directory), ["2026-10-22"]);
    assert_eq!(
        apply_staged(&directory, now(), &TimeZone::UTC).unwrap(),
        None
    );
}

#[test]
fn backups_are_named_on_the_local_clock_and_the_newest_is_found() {
    let (_old, old_pc) = folder(&["2026-03-02"], 450);
    let file = old_pc.path().join("export.worktime");
    export(&old_pc, &file, now()).unwrap();
    let (_new, new_pc) = folder(&["2026-10-22"], 480);
    assert_eq!(latest_backup(&new_pc), None);
    let paris = TimeZone::get("Europe/Paris").unwrap();

    stage(&file, &new_pc).unwrap();
    apply_staged(&new_pc, now(), &paris).unwrap();
    stage(&file, &new_pc).unwrap();
    apply_staged(&new_pc, "2026-10-22T10:00:00Z".parse().unwrap(), &paris).unwrap();

    assert_eq!(
        latest_backup(&new_pc),
        Some(
            new_pc
                .backups_directory()
                .join("before-import-2026-10-22-12-00-00.worktime")
        )
    );
}

#[test]
fn the_days_of_the_ledger_in_place_are_read_without_changing_it() {
    let (_temporary, directory) = folder(&["2026-10-22", "2026-09-01"], 480);
    assert_eq!(
        ledger_days(&directory).unwrap(),
        Some((date(2026, 9, 1), date(2026, 10, 22)))
    );
    let empty = TempDir::new().unwrap();
    assert_eq!(
        ledger_days(&DataDirectory::from(empty.path().to_path_buf())).unwrap(),
        None
    );
}
