use jiff::civil::date;
use jiff::{SignedDuration, Span, Timestamp};

use super::*;
use crate::test_support::TemporaryDatabase;

fn monday() -> Date {
    date(2026, 9, 7)
}

fn plus_days(day: Date, days: i64) -> Date {
    day + Span::new().days(days)
}

/// `Entry` of the .NET tests: a movement of `ticks` created at `created_hour` UTC on its day.
#[allow(clippy::too_many_arguments)]
fn entry(
    id: &str,
    workday_offset: i64,
    kind: CreditLedgerEntryKind,
    ticks: i64,
    origin_offset: i64,
    origin_revision: i64,
    created_hour: i8,
    week: Date,
) -> CreditLedgerEntry {
    let workday = plus_days(week, workday_offset);
    let created_at: Timestamp = workday
        .at(created_hour, 0, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp();
    CreditLedgerEntry::new(
        CreditLedgerEntryId::new(id).unwrap(),
        workday,
        kind,
        from_ticks(ticks),
        CreditOrigin {
            workday_date: plus_days(week, origin_offset),
            revision: origin_revision,
        },
        created_at,
    )
    .unwrap()
}

#[test]
fn all_kinds_and_caller_supplied_provenance_round_trip_at_tick_precision() {
    let database = TemporaryDatabase::new();
    let repository = CreditLedgerRepository::new(&database.connection);
    let supplied = [
        entry(
            " expired ",
            4,
            CreditLedgerEntryKind::Expired,
            4,
            0,
            14,
            16,
            monday(),
        ),
        entry(
            "applied",
            1,
            CreditLedgerEntryKind::Applied,
            1,
            0,
            12,
            10,
            monday(),
        ),
        entry(
            "reversed",
            2,
            CreditLedgerEntryKind::Reversed,
            3,
            1,
            13,
            11,
            monday(),
        ),
        entry(
            "z-earned",
            0,
            CreditLedgerEntryKind::Earned,
            2,
            0,
            11,
            9,
            monday(),
        ),
        entry(
            "a-earned",
            0,
            CreditLedgerEntryKind::Earned,
            5,
            0,
            10,
            9,
            monday(),
        ),
    ];

    repository.replace_week(monday(), &supplied).unwrap();
    let restored = repository.read_week(monday()).unwrap();

    assert_eq!(
        restored,
        [
            supplied[4].clone(),
            supplied[3].clone(),
            supplied[1].clone(),
            supplied[2].clone(),
            supplied[0].clone(),
        ]
    );
    assert_eq!(restored[2].amount(), SignedDuration::from_nanos(100));
    assert_eq!(restored[4].id().as_str(), " expired ");
}

#[test]
fn replacement_is_idempotent_and_empty_replacement_clears_only_its_week() {
    let database = TemporaryDatabase::new();
    let repository = CreditLedgerRepository::new(&database.connection);
    let following_monday = plus_days(monday(), 7);
    let original = entry(
        "original",
        0,
        CreditLedgerEntryKind::Earned,
        10,
        0,
        1,
        9,
        monday(),
    );
    let replacement = entry(
        "replacement",
        1,
        CreditLedgerEntryKind::Applied,
        7,
        0,
        2,
        10,
        monday(),
    );
    let following_week = entry(
        "following-week",
        0,
        CreditLedgerEntryKind::Earned,
        5,
        0,
        3,
        9,
        following_monday,
    );
    repository
        .replace_week(monday(), std::slice::from_ref(&original))
        .unwrap();
    repository
        .replace_week(following_monday, std::slice::from_ref(&following_week))
        .unwrap();

    repository
        .replace_week(monday(), std::slice::from_ref(&replacement))
        .unwrap();
    repository
        .replace_week(monday(), std::slice::from_ref(&replacement))
        .unwrap();

    assert_eq!(repository.read_week(monday()).unwrap(), [replacement]);
    assert_eq!(
        repository.read_week(following_monday).unwrap(),
        std::slice::from_ref(&following_week)
    );

    repository.replace_week(monday(), &[]).unwrap();

    assert_eq!(repository.read_week(monday()).unwrap(), []);
    assert_eq!(
        repository.read_week(following_monday).unwrap(),
        [following_week]
    );
}

#[test]
fn insert_constraint_failure_after_deletion_rolls_back_whole_replacement() {
    let database = TemporaryDatabase::new();
    let repository = CreditLedgerRepository::new(&database.connection);
    let following_monday = plus_days(monday(), 7);
    let existing = entry(
        "target-existing",
        0,
        CreditLedgerEntryKind::Earned,
        10,
        0,
        1,
        9,
        monday(),
    );
    let unrelated = entry(
        "cross-week-id",
        0,
        CreditLedgerEntryKind::Earned,
        5,
        0,
        2,
        9,
        following_monday,
    );
    let conflicting_replacement = entry(
        "cross-week-id",
        1,
        CreditLedgerEntryKind::Applied,
        3,
        0,
        3,
        10,
        monday(),
    );
    let inserted_before_failure = entry(
        "inserted-before-failure",
        0,
        CreditLedgerEntryKind::Earned,
        2,
        0,
        4,
        8,
        monday(),
    );
    repository
        .replace_week(monday(), std::slice::from_ref(&existing))
        .unwrap();
    repository
        .replace_week(following_monday, std::slice::from_ref(&unrelated))
        .unwrap();

    let result = repository.replace_week(
        monday(),
        &[inserted_before_failure, conflicting_replacement],
    );

    assert!(
        matches!(
            result,
            Err(CreditLedgerError::Storage(StorageError::Sqlite(_)))
        ),
        "{result:?}"
    );
    assert_eq!(repository.read_week(monday()).unwrap(), [existing]);
    assert_eq!(repository.read_week(following_monday).unwrap(), [unrelated]);
}

/// The .NET test also replaces with a cancelled token; a synchronous replacement has none.
#[test]
fn invalid_replacements_preserve_existing_week() {
    let database = TemporaryDatabase::new();
    let repository = CreditLedgerRepository::new(&database.connection);
    let existing = entry(
        "existing",
        0,
        CreditLedgerEntryKind::Earned,
        10,
        0,
        1,
        9,
        monday(),
    );
    repository
        .replace_week(monday(), std::slice::from_ref(&existing))
        .unwrap();
    let wrong_week = entry(
        "wrong-week",
        0,
        CreditLedgerEntryKind::Earned,
        1,
        0,
        2,
        9,
        plus_days(monday(), 7),
    );

    let result = repository.replace_week(monday(), &[wrong_week]);

    assert!(
        matches!(&result, Err(CreditLedgerError::EntryOutsideWeek(id)) if id.as_str() == "wrong-week"),
        "{result:?}"
    );
    assert_eq!(repository.read_week(monday()).unwrap(), [existing]);
}

#[test]
fn a_week_starts_on_monday() {
    let database = TemporaryDatabase::new();
    let repository = CreditLedgerRepository::new(&database.connection);
    let tuesday = plus_days(monday(), 1);

    assert!(matches!(
        repository.read_week(tuesday),
        Err(CreditLedgerError::NotAMonday(day)) if day == tuesday
    ));
    assert!(matches!(
        repository.replace_week(tuesday, &[]),
        Err(CreditLedgerError::NotAMonday(day)) if day == tuesday
    ));
}

#[test]
fn created_at_is_read_as_dotnet_parses_it() {
    let database = TemporaryDatabase::new();
    let repository = CreditLedgerRepository::new(&database.connection);
    let stored = entry(
        "stored",
        0,
        CreditLedgerEntryKind::Earned,
        10,
        0,
        1,
        9,
        monday(),
    );
    repository
        .replace_week(monday(), std::slice::from_ref(&stored))
        .unwrap();
    let created_at = |text: &str| {
        database
            .connection
            .execute("UPDATE CreditLedgerEntry SET CreatedAtUtc = ?1;", [text])
            .unwrap();
        repository
            .read_week(monday())
            .map(|entries| format_instant(entries[0].created_at()))
    };

    assert_eq!(
        created_at("2026-09-07T11:00:00+02:00").unwrap(),
        "2026-09-07T09:00:00.0000000+00:00"
    );
    assert_eq!(
        created_at(" 2026-09-07 09:00:00.12345678Z ").unwrap(),
        "2026-09-07T09:00:00.1234568+00:00"
    );
    assert!(matches!(
        created_at("07/09/2026 09:00:00"),
        Err(CreditLedgerError::Storage(
            StorageError::InvalidStoredValue(_)
        ))
    ));
}
