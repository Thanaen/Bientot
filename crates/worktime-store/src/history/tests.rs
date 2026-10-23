use std::path::Path;
use std::sync::Barrier;

use jiff::civil::date;
use jiff::{SignedDuration, Timestamp};
use worktime_domain::accounting::{AccountingLabel, AdjustmentId, LongLockReviewId};
use worktime_domain::history::{
    DailySummaryBreakdown, DailySummaryBreakdownEntry, DailySummaryDisposition,
};
use worktime_domain::intervals::UtcInterval;

use super::*;
use crate::database::open;
use crate::test_support::TemporaryDatabase;

fn monday() -> Date {
    date(2026, 9, 7)
}

fn ticks(ticks: i64) -> SignedDuration {
    from_ticks(ticks)
}

fn at(day: Date, hour: i8) -> Timestamp {
    day.at(hour, 0, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn entry(
    disposition: DailySummaryDisposition,
    label: AccountingLabel,
    duration: i64,
    interval: Option<UtcInterval>,
    adjustment_id: Option<&str>,
    review_id: Option<&str>,
) -> DailySummaryBreakdownEntry {
    DailySummaryBreakdownEntry::new(
        disposition,
        label,
        ticks(duration),
        interval,
        adjustment_id.map(|id| AdjustmentId::new(id).unwrap()),
        review_id.map(|id| LongLockReviewId::new(id).unwrap()),
    )
    .unwrap()
}

/// `Revision` of the .NET tests: every disposition, tick-precise instants and durations.
fn revision(day: Date, revision: i64, fingerprint: &str, complete: bool) -> DailySummaryRevision {
    let start = at(day, 8) + ticks(1);
    let interval = |hours: i64, length: i64| {
        let from = start + SignedDuration::from_hours(hours);
        Some(UtcInterval::new(from, from + ticks(length)).unwrap())
    };
    use AccountingLabel as Label;
    use DailySummaryDisposition as Disposition;
    let entries = [
        entry(
            Disposition::ManualCredit,
            Label::Manual,
            13,
            None,
            Some("adjustment-1"),
            None,
        ),
        entry(
            Disposition::Unknown,
            Label::Unknown,
            11,
            interval(3, 11),
            None,
            None,
        ),
        entry(
            Disposition::Excluded,
            Label::Afk,
            5,
            interval(2, 5),
            None,
            None,
        ),
        entry(
            Disposition::ProvisionalCredit,
            Label::LockedWork,
            3,
            interval(1, 3),
            None,
            Some("review-1"),
        ),
        entry(
            Disposition::ConfirmedCredit,
            Label::ActiveWork,
            5,
            interval(0, 5),
            None,
            None,
        ),
        entry(
            Disposition::Excluded,
            Label::OvernightLock,
            2,
            interval(4, 2),
            None,
            None,
        ),
    ];
    let snapshot = DailySummarySnapshot::new(DailySummarySnapshotParts {
        workday: LocalWorkday::new(day, "Europe/Paris"),
        computed_at: at(day, 12) + ticks(17),
        input_fingerprint: fingerprint.to_owned(),
        credited_work: ticks(21),
        excluded_work: ticks(7),
        unknown_work: ticks(11),
        nominal_target: ticks(if complete { 22 } else { 1_001 }),
        effective_target: ticks(if complete { 21 } else { 1_000 }),
        carry_available: ticks(29),
        carry_applied: ticks(1),
        carry_earned: ticks(31),
        carry_expired: ticks(37),
        carry_debt: ticks(41),
        debt_pending: false,
        projected_departure: (!complete).then(|| at(day, 18) + ticks(19)),
        indicator_state: if complete {
            IndicatorState::Blue
        } else {
            IndicatorState::Green
        },
        source_quality: SourceQuality::Ambiguous,
        has_pending_review: true,
        breakdown: DailySummaryBreakdown::new(entries),
    })
    .unwrap();
    DailySummaryRevision::new(revision, snapshot).unwrap()
}

fn count(connection: &Connection, day: Date) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM DailySummaryRevision WHERE WorkdayDate = ?1;",
            [format_date(day)],
            |row| row.get(0),
        )
        .unwrap()
}

fn sequence_error(result: Result<AppendDailySummaryResult, DailySummaryError>) -> (Date, i64, i64) {
    match result {
        Err(DailySummaryError::RevisionSequence {
            workday_date,
            expected_revision,
            attempted_revision,
        }) => (workday_date, expected_revision, attempted_revision),
        other => panic!("{other:?}"),
    }
}

#[test]
fn complete_typed_snapshot_round_trips_at_tick_precision() {
    for complete in [false, true] {
        let database = TemporaryDatabase::new();
        let repository = DailySummaryRepository::new(&database.connection);
        let supplied = revision(monday(), 1, "all-fields", complete);

        let result = repository.append(&supplied).unwrap();
        let restored = repository.read_latest(monday()).unwrap().unwrap();

        assert_eq!(result.outcome, AppendDailySummaryOutcome::Inserted);
        assert_eq!(restored, supplied);
        assert_eq!(
            restored.snapshot().projected_departure().is_none(),
            complete
        );
        let entries = restored.snapshot().breakdown().entries();
        assert_eq!(entries.len(), 6);
        assert!(entries.iter().any(|entry| {
            entry.disposition() == DailySummaryDisposition::ConfirmedCredit
                && entry.duration() == ticks(5)
                && entry.interval().is_some()
        }));
        assert!(entries.iter().any(|entry| {
            entry.disposition() == DailySummaryDisposition::ProvisionalCredit
                && entry.review_id().map(LongLockReviewId::as_str) == Some("review-1")
        }));
        assert!(entries.iter().any(|entry| {
            entry.disposition() == DailySummaryDisposition::ManualCredit
                && entry.adjustment_id().map(AdjustmentId::as_str) == Some("adjustment-1")
                && entry.interval().is_none()
        }));
        assert!(entries.iter().any(|entry| {
            entry.disposition() == DailySummaryDisposition::Excluded
                && entry.label() == AccountingLabel::OvernightLock
        }));
    }
}

#[test]
fn revisions_advance_in_sequence_and_keep_only_the_latest() {
    let database = TemporaryDatabase::new();
    let repository = DailySummaryRepository::new(&database.connection);
    let first = revision(monday(), 1, "first", false);
    let second = revision(monday(), 2, "second", false);

    assert_eq!(
        repository.append(&first).unwrap().outcome,
        AppendDailySummaryOutcome::Inserted
    );
    assert_eq!(
        repository.append(&first).unwrap(),
        AppendDailySummaryResult {
            outcome: AppendDailySummaryOutcome::AlreadyRecorded,
            authoritative_revision: 1,
        }
    );
    match repository.append(&revision(monday(), 1, "different", false)) {
        Err(DailySummaryError::RevisionConflict {
            workday_date,
            revision,
        }) => assert_eq!((workday_date, revision), (monday(), 1)),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        sequence_error(repository.append(&revision(monday(), 3, "gap", false))),
        (monday(), 2, 3)
    );

    repository.append(&second).unwrap();
    let tuesday = date(2026, 9, 8);
    assert_eq!(
        sequence_error(repository.append(&revision(tuesday, 2, "missing-initial", false))),
        (tuesday, 1, 2)
    );

    assert_eq!(repository.read_latest(monday()).unwrap(), Some(second));
    assert_eq!(count(&database.connection, monday()), 1);
    assert_eq!(sequence_error(repository.append(&first)), (monday(), 3, 1));
}

/// The .NET test also appends with a cancelled token; a synchronous append has none.
#[test]
fn invalid_sequence_leaves_existing_history_unchanged() {
    let database = TemporaryDatabase::new();
    let repository = DailySummaryRepository::new(&database.connection);
    let first = revision(monday(), 1, "first", false);
    repository.append(&first).unwrap();

    sequence_error(repository.append(&revision(monday(), 3, "gap", false)));

    assert_eq!(repository.read_latest(monday()).unwrap(), Some(first));
    assert_eq!(count(&database.connection, monday()), 1);
}

/// Appends each revision from its own thread and connection, all at once.
fn append_concurrently(
    path: &Path,
    revisions: &[DailySummaryRevision],
) -> Vec<Result<AppendDailySummaryResult, DailySummaryError>> {
    let start = Barrier::new(revisions.len());
    std::thread::scope(|scope| {
        let workers: Vec<_> = revisions
            .iter()
            .map(|revision| {
                let start = &start;
                scope.spawn(move || {
                    let connection = open(path).unwrap();
                    start.wait();
                    DailySummaryRepository::new(&connection).append(revision)
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect()
    })
}

#[test]
fn concurrent_exact_retries_produce_one_insert_and_one_already_recorded() {
    let database = TemporaryDatabase::new();
    let same = revision(monday(), 1, "same", false);

    let mut outcomes: Vec<_> = append_concurrently(&database.path(), &[same.clone(), same])
        .into_iter()
        .map(|result| result.unwrap().outcome)
        .collect();
    outcomes.sort_by_key(|outcome| *outcome == AppendDailySummaryOutcome::AlreadyRecorded);

    assert_eq!(
        outcomes,
        [
            AppendDailySummaryOutcome::Inserted,
            AppendDailySummaryOutcome::AlreadyRecorded
        ]
    );
    assert_eq!(count(&database.connection, monday()), 1);
}

#[test]
fn concurrent_different_contents_produce_one_insert_and_one_typed_conflict() {
    let database = TemporaryDatabase::new();

    let results = append_concurrently(
        &database.path(),
        &[
            revision(monday(), 1, "left", false),
            revision(monday(), 1, "right", false),
        ],
    );

    let inserted = results
        .iter()
        .filter(|result| {
            matches!(result, Ok(result) if result.outcome == AppendDailySummaryOutcome::Inserted)
        })
        .count();
    let conflicts = results
        .iter()
        .filter(|result| matches!(result, Err(DailySummaryError::RevisionConflict { .. })))
        .count();
    assert_eq!((inserted, conflicts), (1, 1));
    assert_eq!(count(&database.connection, monday()), 1);
}

#[test]
fn transaction_aware_append_does_not_commit_its_caller_transaction() {
    let database = TemporaryDatabase::new();
    let connection = database.reopen();
    let transaction =
        rusqlite::Transaction::new_unchecked(&connection, TransactionBehavior::Immediate).unwrap();

    let result = DailySummaryRepository::append_within(
        &transaction,
        &revision(monday(), 1, "rolled-back", false),
    )
    .unwrap();
    transaction.rollback().unwrap();

    assert_eq!(result.outcome, AppendDailySummaryOutcome::Inserted);
    assert_eq!(
        DailySummaryRepository::new(&database.connection)
            .read_latest(monday())
            .unwrap(),
        None
    );
}

#[test]
fn latest_week_uses_containing_week_and_returns_only_latest_ordered_weekdays() {
    let database = TemporaryDatabase::new();
    let repository = DailySummaryRepository::new(&database.connection);
    let year_boundary_monday = date(2026, 12, 28);
    let day = |offset: i64| year_boundary_monday + jiff::Span::new().days(offset);
    for (workday, number, fingerprint) in [
        (day(0), 1, "monday-first"),
        (day(0), 2, "monday-second"),
        (day(2), 1, "wednesday"),
        (day(4), 1, "friday"),
        (day(7), 1, "outside"),
    ] {
        repository
            .append(&revision(workday, number, fingerprint, false))
            .unwrap();
    }

    let restored = repository.read_latest_week(date(2027, 1, 3)).unwrap();

    let summary: Vec<_> = restored
        .iter()
        .map(|revision| {
            (
                revision.workday_date(),
                revision.revision(),
                revision.snapshot().input_fingerprint(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (day(0), 2, "monday-second"),
            (day(2), 1, "wednesday"),
            (day(4), 1, "friday"),
        ]
    );
}

#[test]
fn unsupported_or_malformed_stored_values_fail_explicitly() {
    let cases = [
        ("ComputedAtUtc", "2026-09-07"),
        ("ProjectedDepartureUtc", "2026-09-07T18:00:00.0000000"),
        ("BreakdownJson", "{}"),
        ("BreakdownJson", r#"{"version":2,"entries":[]}"#),
        ("BreakdownJson", "{not-json"),
        (
            "BreakdownJson",
            r#"{"version":1,"entries":[{"disposition":"future","label":"active-work","durationTicks":5,"startUtc":null,"endUtc":null,"adjustmentId":null,"reviewId":null}]}"#,
        ),
        ("IndicatorState", "purple"),
    ];
    for (column, value) in cases {
        let database = TemporaryDatabase::new();
        let repository = DailySummaryRepository::new(&database.connection);
        repository
            .append(&revision(monday(), 1, "valid", false))
            .unwrap();
        database
            .connection
            .execute(
                &format!("UPDATE DailySummaryRevision SET {column} = ?1;"),
                [value],
            )
            .unwrap();

        let result = repository.read_latest(monday());

        assert!(
            matches!(
                result,
                Err(DailySummaryError::Storage(
                    StorageError::InvalidStoredValue(_)
                ))
            ),
            "{column} = {value}: {result:?}"
        );
    }
}

#[test]
fn a_sub_tick_revision_is_stored_at_tick_precision_and_its_retry_is_already_recorded() {
    let database = TemporaryDatabase::new();
    let repository = DailySummaryRepository::new(&database.connection);
    let at_ticks = revision(monday(), 1, "sub-tick", true);
    let parts = |computed_at| {
        let snapshot = at_ticks.snapshot();
        DailySummarySnapshotParts {
            workday: snapshot.workday().clone(),
            computed_at,
            input_fingerprint: snapshot.input_fingerprint().to_owned(),
            credited_work: snapshot.credited_work(),
            excluded_work: snapshot.excluded_work(),
            unknown_work: snapshot.unknown_work(),
            nominal_target: snapshot.nominal_target(),
            effective_target: snapshot.effective_target(),
            carry_available: snapshot.carry_available(),
            carry_applied: snapshot.carry_applied(),
            carry_earned: snapshot.carry_earned(),
            carry_expired: snapshot.carry_expired(),
            carry_debt: snapshot.carry_debt(),
            debt_pending: snapshot.debt_pending(),
            projected_departure: snapshot.projected_departure(),
            indicator_state: snapshot.indicator_state(),
            source_quality: snapshot.source_quality(),
            has_pending_review: snapshot.has_pending_review(),
            breakdown: snapshot.breakdown().clone(),
        }
    };
    let sub_tick = |nanos| {
        let computed_at = at_ticks.snapshot().computed_at() + SignedDuration::from_nanos(nanos);
        DailySummaryRevision::new(1, DailySummarySnapshot::new(parts(computed_at)).unwrap())
            .unwrap()
    };

    let first = repository.append(&sub_tick(42)).unwrap();
    let retry = repository.append(&sub_tick(99)).unwrap();

    assert_eq!(first.outcome, AppendDailySummaryOutcome::Inserted);
    assert_eq!(retry.outcome, AppendDailySummaryOutcome::AlreadyRecorded);
    assert_eq!(repository.read_latest(monday()).unwrap(), Some(at_ticks));
}

#[test]
fn a_revision_that_is_invalid_at_tick_precision_is_refused() {
    let database = TemporaryDatabase::new();
    let repository = DailySummaryRepository::new(&database.connection);
    // Two entries of 300 ticks and a half add up to 601 ticks, but to 600 once truncated.
    let half_tick = SignedDuration::from_nanos(50);
    let part = || {
        DailySummaryBreakdownEntry::new(
            DailySummaryDisposition::ConfirmedCredit,
            AccountingLabel::ActiveWork,
            ticks(300) + half_tick,
            None,
            None,
            None,
        )
        .unwrap()
    };
    let snapshot = DailySummarySnapshot::new(DailySummarySnapshotParts {
        workday: LocalWorkday::new(monday(), "Europe/Paris"),
        computed_at: at(monday(), 12),
        input_fingerprint: "sub-tick".to_owned(),
        credited_work: ticks(601),
        excluded_work: SignedDuration::ZERO,
        unknown_work: SignedDuration::ZERO,
        nominal_target: ticks(1_000),
        effective_target: ticks(1_000),
        carry_available: SignedDuration::ZERO,
        carry_applied: SignedDuration::ZERO,
        carry_earned: SignedDuration::ZERO,
        carry_expired: SignedDuration::ZERO,
        carry_debt: SignedDuration::ZERO,
        debt_pending: false,
        projected_departure: Some(at(monday(), 18)),
        indicator_state: IndicatorState::Green,
        source_quality: SourceQuality::Complete,
        has_pending_review: false,
        breakdown: DailySummaryBreakdown::new([part(), part()]),
    })
    .unwrap();

    let result = repository.append(&DailySummaryRevision::new(1, snapshot).unwrap());

    assert!(
        matches!(result, Err(DailySummaryError::NotStorable(_))),
        "{result:?}"
    );
    assert_eq!(count(&database.connection, monday()), 0);
}
