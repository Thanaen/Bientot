//! A `.bientot` file exported on one platform imports on the other without loss. CI runs these
//! tests on Linux and on Windows, so each export below is read on both. The two exports hold the
//! same week worked in Paris, as each platform writes it; `write_the_fixtures` in
//! `bientot-app/tests/cross_platform_data.rs` writes them again.

use std::path::PathBuf;

use bientot_domain::accounting::{OperationKey, PowerState, RecordedTransition};
use bientot_domain::history::DailySummaryRevision;
use bientot_domain::time::{FrozenClock, LocalWorkday, WorkdayWindow};
use bientot_store::data_directory::DataDirectory;
use bientot_store::history::DailySummaryRepository;
use bientot_store::sessions::SessionEventRepository;
use bientot_store::transfer;
use jiff::civil::{Date, date};
use jiff::tz::TimeZone;
use rusqlite::Connection;

const IMPORTED: &str = "2026-10-09T16:00:00Z";

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// The ledger of a new data folder once the next start applied the import of `name`.
fn imported(name: &str) -> (tempfile::TempDir, Connection) {
    let temporary = tempfile::tempdir().unwrap();
    let directory = DataDirectory::from(temporary.path().join("data"));
    transfer::stage(&fixture(name), &directory).unwrap();
    let applied = transfer::apply_staged(&directory, IMPORTED.parse().unwrap(), &TimeZone::UTC);
    assert!(applied.unwrap().is_some());
    let connection = bientot_store::database::open(&directory.database_file()).unwrap();
    bientot_store::database::migrate(&connection).unwrap();
    (temporary, connection)
}

fn weekdays() -> impl Iterator<Item = Date> {
    (5..10).map(|day| date(2026, 10, day))
}

fn latest(connection: &Connection, day: Date) -> DailySummaryRevision {
    DailySummaryRepository::new(connection)
        .read_latest(day)
        .unwrap()
        .unwrap_or_else(|| panic!("no summary of {day}"))
}

#[test]
fn a_week_from_either_platform_imports_with_the_same_summaries() {
    let (_windows_folder, windows) = imported("export-windows.bientot");
    let (_linux_folder, linux) = imported("export-linux.bientot");

    for day in weekdays() {
        let from_windows = latest(&windows, day);
        let from_linux = latest(&linux, day);
        let [windows_day, linux_day] = [&from_windows, &from_linux].map(|revision| {
            let summary = revision.snapshot();
            (
                (
                    summary.credited_work(),
                    summary.excluded_work(),
                    summary.unknown_work(),
                    summary.nominal_target(),
                    summary.effective_target(),
                    summary.projected_departure(),
                    summary.indicator_state(),
                    summary.source_quality(),
                    summary.has_pending_review(),
                ),
                (
                    summary.carry_available(),
                    summary.carry_applied(),
                    summary.carry_earned(),
                    summary.carry_expired(),
                    summary.carry_debt(),
                    summary.debt_pending(),
                ),
                summary.breakdown().clone(),
            )
        });

        assert_eq!(windows_day, linux_day, "{day}");
        assert_eq!(
            from_windows.snapshot().workday(),
            &LocalWorkday::new(day, "Romance Standard Time")
        );
        assert_eq!(
            from_linux.snapshot().workday(),
            &LocalWorkday::new(day, "Europe/Paris")
        );
    }
}

#[test]
fn the_power_history_of_either_platform_imports_without_loss() {
    let wednesday = WorkdayWindow {
        workday: LocalWorkday::new(date(2026, 10, 7), "Europe/Paris"),
        start: "2026-10-06T22:00:00Z".parse().unwrap(),
        end: "2026-10-07T22:00:00Z".parse().unwrap(),
    };
    let clock = FrozenClock::new(IMPORTED.parse().unwrap());
    let histories = ["export-windows.bientot", "export-linux.bientot"].map(|name| {
        let (_folder, connection) = imported(name);
        let events = SessionEventRepository::new(&connection, &clock);
        let keys: Vec<String> = connection
            .prepare("SELECT OperationKey FROM SessionEvent;")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(keys.len(), 4, "{name}: {keys:#?}");
        for key in &keys {
            let found = events.find(&OperationKey::new(key.as_str()).unwrap());
            assert!(found.unwrap().is_some(), "{name}: {key}");
        }
        events.read_for_accounting(&wednesday).unwrap()
    });

    assert_eq!(histories[0], histories[1]);
    let suspended = histories[1].in_window().iter().any(|transition| {
        matches!(transition, RecordedTransition::Power(recorded)
            if recorded.state == PowerState::Suspended
                && recorded.occurred_at == "2026-10-07T10:35:00Z".parse().unwrap())
    });
    assert!(suspended, "{:#?}", histories[1]);
}
