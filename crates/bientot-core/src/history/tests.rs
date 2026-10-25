//! Ported from `HistoryServiceTests.cs` and `HistoryPersistenceTests.cs`, against a migrated
//! SQLite ledger instead of a recording store.

use bientot_domain::accounting::{AccountingLabel, LongLockReviewId};
use bientot_domain::history::{
    DailySummaryBreakdown, DailySummaryBreakdownEntry, DailySummaryDisposition,
    DailySummaryRevision, DailySummarySnapshot, DailySummarySnapshotParts,
};
use bientot_domain::intervals::UtcInterval;
use bientot_domain::sources::SourceQuality;
use bientot_domain::summaries::IndicatorState;
use bientot_domain::time::{FixedTimeZoneProvider, FrozenClock, LocalWorkday};
use bientot_store::StorageError;
use jiff::civil::{Date, date};
use jiff::tz::{Offset, TimeZone};
use jiff::{SignedDuration, Timestamp};

use super::*;
use crate::test_support::{CountingClock, Database, utc, utc_zones};

const TICK: SignedDuration = SignedDuration::from_nanos(100);

/// The revision the .NET `Revision` helper builds: one tick of provisional credit awaiting
/// review, in `time_zone_id`.
fn revision(day: Date, revision: i64, time_zone_id: &str) -> DailySummaryRevision {
    let start = utc(day.year(), day.month(), day.day(), 8);
    let interval = UtcInterval::new(start, start + TICK).unwrap();
    let snapshot = DailySummarySnapshot::new(DailySummarySnapshotParts {
        workday: LocalWorkday::new(day, time_zone_id),
        computed_at: utc(day.year(), day.month(), day.day(), 12),
        input_fingerprint: format!("fingerprint-{}-{revision}", day.strftime("%Y%m%d")),
        credited_work: TICK,
        excluded_work: SignedDuration::ZERO,
        unknown_work: SignedDuration::ZERO,
        nominal_target: TICK * 2,
        effective_target: TICK * 2,
        carry_available: SignedDuration::ZERO,
        carry_applied: SignedDuration::ZERO,
        carry_earned: SignedDuration::ZERO,
        carry_expired: SignedDuration::ZERO,
        carry_debt: SignedDuration::ZERO,
        debt_pending: false,
        projected_departure: Some(utc(day.year(), day.month(), day.day(), 18)),
        indicator_state: IndicatorState::Green,
        source_quality: SourceQuality::Ambiguous,
        has_pending_review: true,
        breakdown: DailySummaryBreakdown::new([DailySummaryBreakdownEntry::new(
            DailySummaryDisposition::ProvisionalCredit,
            AccountingLabel::LockedWork,
            TICK,
            Some(interval),
            None,
            Some(LongLockReviewId::new("review-1").unwrap()),
        )
        .unwrap()]),
    })
    .unwrap();
    DailySummaryRevision::new(revision, snapshot).unwrap()
}

/// The summary `HistoryPersistenceTests` stores: `worked` minutes, complete, in Paris.
fn worked(day: Date, revision: i64, worked: i64) -> DailySummaryRevision {
    let worked_time = SignedDuration::from_mins(worked);
    let snapshot = DailySummarySnapshot::new(DailySummarySnapshotParts {
        workday: LocalWorkday::new(day, "Europe/Paris"),
        computed_at: utc(day.year(), day.month(), day.day(), 16),
        input_fingerprint: format!("worked:{worked}"),
        credited_work: worked_time,
        excluded_work: SignedDuration::ZERO,
        unknown_work: SignedDuration::ZERO,
        nominal_target: SignedDuration::from_mins(420),
        effective_target: SignedDuration::from_mins(420),
        carry_available: SignedDuration::ZERO,
        carry_applied: SignedDuration::ZERO,
        carry_earned: SignedDuration::ZERO,
        carry_expired: SignedDuration::ZERO,
        carry_debt: SignedDuration::ZERO,
        debt_pending: false,
        projected_departure: None,
        indicator_state: IndicatorState::Blue,
        source_quality: SourceQuality::Complete,
        has_pending_review: false,
        breakdown: DailySummaryBreakdown::new([DailySummaryBreakdownEntry::new(
            DailySummaryDisposition::ConfirmedCredit,
            AccountingLabel::ActiveWork,
            worked_time,
            None,
            None,
            None,
        )
        .unwrap()]),
    })
    .unwrap();
    DailySummaryRevision::new(revision, snapshot).unwrap()
}

/// Appends revisions 1 to `latest`, the last one being `latest`, since the ledger only takes the
/// next revision of a day.
fn store_up_to(database: &Database, latest: &DailySummaryRevision) {
    let repository = DailySummaryRepository::new(&database.connection);
    let snapshot = latest.snapshot();
    for number in 1..latest.revision() {
        repository
            .append(&revision(
                latest.workday_date(),
                number,
                &snapshot.workday().time_zone_id,
            ))
            .unwrap();
    }
    repository.append(latest).unwrap();
}

fn current_week_at(database: &Database, now: Timestamp) -> CurrentWeek {
    let clock = FrozenClock::new(now);
    let zones = utc_zones();
    HistoryService::new(
        DailySummaryRepository::new(&database.connection),
        &clock,
        &zones,
    )
    .current_week()
    .unwrap()
}

#[test]
fn uses_current_local_date_from_one_clock_reading() {
    let database = Database::new();
    store_up_to(&database, &revision(date(2026, 12, 28), 1, "UTC"));
    let monday = revision(date(2027, 1, 4), 1, "UTC");
    store_up_to(&database, &monday);
    let clock = CountingClock::new(utc(2027, 1, 3, 22) + SignedDuration::from_mins(30));
    let zones = FixedTimeZoneProvider::new(
        "Test UTC+02",
        TimeZone::fixed(Offset::from_hours(2).unwrap()),
    );

    let week = HistoryService::new(
        DailySummaryRepository::new(&database.connection),
        &clock,
        &zones,
    )
    .current_week()
    .unwrap();

    assert_eq!(week.week_start_date(), date(2027, 1, 4));
    assert_eq!(week.days()[0].latest_revision, Some(monday));
    assert_eq!(clock.reads.get(), 1);
}

#[test]
fn weekend_at_year_boundary_returns_five_ordered_slots_with_missing_days() {
    let database = Database::new();
    let monday = revision(date(2026, 12, 28), 2, "Historical Monday");
    let wednesday = revision(date(2026, 12, 30), 1, "Historical Wednesday");
    let friday = revision(date(2027, 1, 1), 3, "Historical Friday");
    for latest in [&friday, &monday, &wednesday] {
        store_up_to(&database, latest);
    }

    let week = current_week_at(&database, utc(2027, 1, 3, 12));

    assert_eq!(week.week_start_date(), date(2026, 12, 28));
    assert_eq!(
        week.days().iter().map(|day| day.date).collect::<Vec<_>>(),
        [
            date(2026, 12, 28),
            date(2026, 12, 29),
            date(2026, 12, 30),
            date(2026, 12, 31),
            date(2027, 1, 1)
        ]
    );
    assert_eq!(week.days()[0].latest_revision, Some(monday));
    assert_eq!(week.days()[1].latest_revision, None);
    assert_eq!(week.days()[2].latest_revision, Some(wednesday));
    assert_eq!(week.days()[3].latest_revision, None);
    assert_eq!(week.days()[4].latest_revision, Some(friday));
}

#[test]
fn keeps_revision_from_a_stored_timezone_different_from_current_machine_timezone() {
    let database = Database::new();
    let persisted = revision(date(2026, 9, 7), 4, "Pacific Standard Time");
    store_up_to(&database, &persisted);

    let week = current_week_at(&database, utc(2026, 9, 9, 12));

    let restored = week.days()[0].latest_revision.as_ref().unwrap();
    assert_eq!(restored, &persisted);
    assert_eq!(
        restored.snapshot().workday().time_zone_id,
        "Pacific Standard Time"
    );
    assert_eq!(
        restored.snapshot().source_quality(),
        SourceQuality::Ambiguous
    );
    assert!(restored.snapshot().has_pending_review());
    assert_eq!(restored.snapshot().breakdown().entries().len(), 1);
}

#[test]
fn explicit_stored_data_failure_is_not_presented_as_empty_history() {
    let database = Database::new();
    store_up_to(&database, &revision(date(2026, 9, 7), 1, "UTC"));
    database
        .connection
        .execute(
            "UPDATE DailySummaryRevision SET BreakdownJson = '{\"version\":2,\"entries\":[]}';",
            [],
        )
        .unwrap();
    let clock = FrozenClock::new(utc(2026, 9, 9, 12));
    let zones = utc_zones();

    let failure = HistoryService::new(
        DailySummaryRepository::new(&database.connection),
        &clock,
        &zones,
    )
    .current_week()
    .unwrap_err();

    let DailySummaryError::Storage(StorageError::InvalidStoredValue(invalid)) = failure else {
        panic!("expected an unreadable row, got {failure:?}");
    };
    assert_eq!(invalid.table, "DailySummaryRevision");
    assert_eq!(
        invalid.detail,
        "Unsupported daily summary breakdown version 2."
    );
}

#[test]
fn reopened_service_reads_latest_revisions_and_preserves_missing_days_within_local_week() {
    let database = Database::new();
    let monday = date(2026, 12, 28);
    let friday = date(2027, 1, 1);
    let now = utc(2027, 1, 3, 12);
    let empty = current_week_at(&database, now);
    assert_eq!(empty.days().len(), 5);
    assert!(empty.days().iter().all(|day| day.latest_revision.is_none()));
    let repository = DailySummaryRepository::new(&database.connection);
    for summary in [
        worked(date(2026, 12, 21), 1, 420),
        worked(monday, 1, 420),
        worked(monday, 2, 450),
        worked(friday, 1, 430),
        worked(date(2027, 1, 4), 1, 440),
    ] {
        repository.append(&summary).unwrap();
    }

    let reopened = database.reopen();
    let clock = FrozenClock::new(now);
    let zones = utc_zones();
    let week = HistoryService::new(DailySummaryRepository::new(&reopened), &clock, &zones)
        .current_week()
        .unwrap();

    assert_eq!(week.week_start_date(), monday);
    let latest_monday = week.days()[0].latest_revision.as_ref().unwrap();
    assert_eq!(latest_monday.revision(), 2);
    assert_eq!(
        latest_monday.snapshot().credited_work(),
        SignedDuration::from_mins(450)
    );
    assert_eq!(
        latest_monday.snapshot().workday().time_zone_id,
        "Europe/Paris"
    );
    assert_eq!(latest_monday.snapshot().input_fingerprint(), "worked:450");
    assert_eq!(
        latest_monday
            .snapshot()
            .breakdown()
            .entries()
            .iter()
            .map(DailySummaryBreakdownEntry::duration)
            .collect::<Vec<_>>(),
        [SignedDuration::from_mins(450)]
    );
    assert!(
        week.days()[1..4]
            .iter()
            .all(|day| day.latest_revision.is_none())
    );
    let latest_friday = week.days()[4].latest_revision.as_ref().unwrap();
    assert_eq!(latest_friday.workday_date(), friday);
    assert_eq!(
        latest_friday.snapshot().credited_work(),
        SignedDuration::from_mins(430)
    );
}
