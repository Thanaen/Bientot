use std::path::Path;
use std::sync::Barrier;

use jiff::civil::date;
use jiff::tz::TimeZone;
use worktime_domain::accounting::{
    AccountingInput, AccountingPolicy, ActivityCoverage, AutomaticOrigin, WorkdayEventHistory,
    calculate,
};

use super::*;
use crate::database::open;
use crate::test_support::TemporaryDatabase;

const ZONE: &str = "Romance Standard Time";

fn workday() -> LocalWorkday {
    LocalWorkday::new(date(2026, 8, 28), ZONE)
}

fn id(value: &str) -> AdjustmentId {
    AdjustmentId::new(value).unwrap()
}

fn at(hour: i8, minute: i8) -> Timestamp {
    on(date(2026, 8, 28), hour, minute)
}

fn on(day: jiff::civil::Date, hour: i8, minute: i8) -> Timestamp {
    day.at(hour, minute, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn between(start_hour: i8, end_hour: i8) -> UtcInterval {
    UtcInterval::new(at(start_hour, 0), at(end_hour, 0)).unwrap()
}

fn duration(
    value: &str,
    workday: LocalWorkday,
    minutes: i64,
    created_at: Timestamp,
) -> ManualAdjustment {
    ManualAdjustment::duration_work(
        id(value),
        workday,
        SignedDuration::from_mins(minutes),
        created_at,
    )
    .unwrap()
}

fn interval_work(value: &str, interval: UtcInterval, created_at: Timestamp) -> ManualAdjustment {
    ManualAdjustment::interval_work(id(value), workday(), interval, created_at).unwrap()
}

fn exclusion(value: &str, interval: UtcInterval, created_at: Timestamp) -> ManualAdjustment {
    ManualAdjustment::exclusion(id(value), workday(), interval, created_at).unwrap()
}

fn revocation(value: &str, target: &str, created_at: Timestamp) -> ManualAdjustment {
    ManualAdjustment::revocation(id(value), workday(), id(target), created_at).unwrap()
}

fn review(value: &str) -> LongLockReviewId {
    LongLockReviewId::new(value).unwrap()
}

fn account(adjustments: Vec<ManualAdjustment>) -> AccountingResult {
    let bounds = between(7, 17);
    let window = worktime_domain::time::WorkdayWindow {
        workday: workday(),
        start: bounds.start(),
        end: bounds.end(),
    };
    calculate(
        &AccountingInput::new(
            window,
            bounds.end(),
            AccountingPolicy::new(between(12, 14), SignedDuration::from_mins(90)).unwrap(),
            ActivityCoverage::unknown(
                bounds,
                AutomaticOrigin::new("legacy-or-concurrency-test").unwrap(),
            ),
            WorkdayEventHistory::default(),
            adjustments,
        )
        .unwrap(),
    )
    .unwrap()
}

fn inserted(result: Result<AppendAdjustmentResult, ManualAdjustmentError>, expected: &str) {
    let result = result.unwrap();
    assert_eq!(
        (result.outcome, result.authoritative_id),
        (AppendAdjustmentOutcome::Inserted, id(expected))
    );
}

fn insert_raw(
    connection: &Connection,
    id: &str,
    kind: &str,
    start_utc: Option<&str>,
    duration_minutes: Option<i64>,
    supersedes_id: Option<&str>,
) -> rusqlite::Result<usize> {
    connection.execute(
        "INSERT INTO ManualAdjustment (
    Id, WorkdayDate, Kind, StartUtc, EndUtc, DurationMinutes, CreatedAtUtc, SupersedesId, TimezoneId)
VALUES (
    $id, '2026-08-28', $kind, $startUtc, $startUtc, $durationMinutes,
    '2026-08-28T14:00:00.0000000+00:00', $supersedesId, 'Romance Standard Time');",
        named_params! {
            "$id": id,
            "$kind": kind,
            "$startUtc": start_utc,
            "$durationMinutes": duration_minutes,
            "$supersedesId": supersedes_id,
        },
    )
}

fn read_reopened(database: &TemporaryDatabase, workday: &LocalWorkday) -> Vec<ManualAdjustment> {
    let connection = database.reopen();
    ManualAdjustmentRepository::new(&connection)
        .read_for_workday(workday)
        .unwrap()
}

#[test]
fn all_closed_variants_round_trip_exactly_after_database_reopen() {
    let database = TemporaryDatabase::new();
    let repository = ManualAdjustmentRepository::new(&database.connection);
    let adjustments = [
        duration("duration", workday(), 35, at(12, 0)).with_reason("Missing meeting"),
        interval_work("interval", between(7, 8), at(12, 0)).with_reason("Early start"),
        exclusion("exclude", between(8, 9), at(12, 1)).with_reason("Personal appointment"),
        ManualAdjustment::long_lock_confirmation(
            id("reclassify"),
            workday(),
            between(9, 10),
            at(12, 2),
            review("session-lock:42"),
        )
        .unwrap()
        .superseding(id("exclude"))
        .with_reason("Confirmed customer call"),
    ];

    for adjustment in &adjustments {
        inserted(repository.append(adjustment), adjustment.id().as_str());
    }

    assert_eq!(read_reopened(&database, &workday()), adjustments);
}

#[test]
fn the_latest_creation_spans_every_workday() {
    let database = TemporaryDatabase::new();
    let repository = ManualAdjustmentRepository::new(&database.connection);
    assert_eq!(repository.latest_created_at().unwrap(), None);
    let later = on(date(2026, 8, 31), 9, 0);
    for adjustment in [
        interval_work("interval", between(7, 8), at(12, 0)),
        duration(
            "monday",
            LocalWorkday::new(date(2026, 8, 31), ZONE),
            5,
            later,
        ),
        exclusion("exclude", between(8, 9), at(12, 1)),
    ] {
        repository.append(&adjustment).unwrap();
    }

    assert_eq!(repository.latest_created_at().unwrap(), Some(later));
}

#[test]
fn a_revocation_round_trips_and_a_target_can_be_revoked_only_once() {
    let database = TemporaryDatabase::new();
    let repository = ManualAdjustmentRepository::new(&database.connection);
    let target = interval_work("interval", between(7, 8), at(12, 0));
    let revoke = revocation("revoke", "interval", at(12, 1)).with_reason("Mistake");
    repository.append(&target).unwrap();

    let result = repository.append(&revoke);
    let again = repository.append(&revocation("revoke-again", "interval", at(12, 2)));

    inserted(result, "revoke");
    assert!(
        matches!(
            &again,
            Err(ManualAdjustmentError::SupersessionConflict { superseded_id, existing_successor_id })
                if (superseded_id, existing_successor_id) == (&id("interval"), &id("revoke"))
        ),
        "{again:?}"
    );
    assert_eq!(read_reopened(&database, &workday()), [target, revoke]);
}

#[test]
fn migrating_a_v9_database_keeps_every_adjustment_and_its_constraints() {
    let database = TemporaryDatabase::at_version(9);
    let existing = vec![
        interval_work("interval", between(7, 8), at(12, 0)).with_reason("Early start"),
        duration("duration", workday(), 35, at(12, 1)),
        exclusion("exclude", between(8, 9), at(12, 2)),
        ManualAdjustment::exclusion(
            id("a-successor-sorted-first"),
            workday(),
            between(8, 10),
            at(12, 3),
        )
        .unwrap()
        .superseding(id("exclude")),
        ManualAdjustment::long_lock_confirmation(
            id("confirm"),
            workday(),
            between(10, 11),
            at(12, 4),
            review("session-lock:42"),
        )
        .unwrap(),
    ];
    let v9 = ManualAdjustmentRepository::new(&database.connection);
    for adjustment in &existing {
        v9.append(adjustment).unwrap();
    }

    assert_eq!(database.migrate(), 11);

    let connection = &database.connection;
    for (id, kind, duration_minutes, supersedes_id) in [
        ("second-successor", "add-work", Some(15), Some("exclude")),
        ("malformed", "add-work", None, None),
        ("revoke-nothing", "revoke", None, None),
        ("revoke-unknown", "revoke", None, Some("unknown")),
        ("revoke-with-duration", "revoke", Some(15), Some("duration")),
    ] {
        let result = insert_raw(connection, id, kind, None, duration_minutes, supersedes_id);
        assert!(
            matches!(result, Err(rusqlite::Error::SqliteFailure(..))),
            "{id}: {result:?}"
        );
    }
    assert_eq!(read_reopened(&database, &workday()), existing);
    let reopened = database.reopen();
    let revoke = revocation("revoke", "interval", at(13, 0));
    inserted(
        ManualAdjustmentRepository::new(&reopened).append(&revoke),
        "revoke",
    );
    assert_eq!(
        read_reopened(&database, &workday()),
        [existing, vec![revoke]].concat()
    );
}

#[test]
fn exact_retry_is_idempotent_but_different_payload_for_same_id_is_a_typed_conflict() {
    let database = TemporaryDatabase::new();
    let repository = ManualAdjustmentRepository::new(&database.connection);
    let original = duration("duration", workday(), 35, at(12, 0)).with_reason("Original reason");

    let first = repository.append(&original).unwrap();
    let retry = repository.append(&original).unwrap();
    let conflicting = original.clone().with_reason("A different payload");
    let conflict = repository.append(&conflicting);

    assert_eq!(first.outcome, AppendAdjustmentOutcome::Inserted);
    assert_eq!(retry.outcome, AppendAdjustmentOutcome::AlreadyRecorded);
    assert_eq!(retry.authoritative_id, id("duration"));
    assert!(
        matches!(&conflict, Err(ManualAdjustmentError::IdConflict(conflict)) if *conflict == id("duration")),
        "{conflict:?}"
    );
    assert_eq!(database.count("ManualAdjustment"), 1);
}

#[test]
fn one_successor_constraint_is_reported_as_a_typed_conflict() {
    let database = TemporaryDatabase::new();
    let repository = ManualAdjustmentRepository::new(&database.connection);
    let original = duration("root", workday(), 30, at(12, 0));
    let first_successor = duration("successor-a", workday(), 40, at(12, 1)).superseding(id("root"));
    let competing_successor =
        duration("successor-b", workday(), 50, at(12, 2)).superseding(id("root"));
    repository.append(&original).unwrap();
    repository.append(&first_successor).unwrap();

    let result = repository.append(&competing_successor);

    assert!(
        matches!(
            &result,
            Err(ManualAdjustmentError::SupersessionConflict { superseded_id, existing_successor_id })
                if (superseded_id, existing_successor_id) == (&id("root"), &id("successor-a"))
        ),
        "{result:?}"
    );
    assert_eq!(database.count("ManualAdjustment"), 2);
}

fn assert_parent_conflict(
    result: Result<AppendAdjustmentResult, ManualAdjustmentError>,
    expected_id: &str,
    expected_reason: ParentConflictReason,
) {
    assert!(
        matches!(
            &result,
            Err(ManualAdjustmentError::ParentConflict { superseded_id, reason })
                if *superseded_id == id(expected_id) && *reason == expected_reason
        ),
        "{result:?}"
    );
}

#[test]
fn missing_superseded_adjustment_is_a_typed_parent_conflict() {
    let database = TemporaryDatabase::new();
    let repository = ManualAdjustmentRepository::new(&database.connection);
    let child = duration("child", workday(), 30, at(12, 1)).superseding(id("missing-parent"));

    let result = repository.append(&child);

    assert_parent_conflict(result, "missing-parent", ParentConflictReason::NotFound);
    assert_eq!(database.count("ManualAdjustment"), 0);
}

#[test]
fn different_date_cannot_create_an_orphaned_child_in_a_reopened_workday_read() {
    let database = TemporaryDatabase::new();
    let repository = ManualAdjustmentRepository::new(&database.connection);
    let parent = duration("parent", workday(), 30, at(12, 0));
    repository.append(&parent).unwrap();
    let child_workday = LocalWorkday::new(date(2026, 8, 31), ZONE);
    let child = duration(
        "child",
        child_workday.clone(),
        40,
        on(date(2026, 8, 31), 12, 0),
    )
    .superseding(id("parent"));

    let result = repository.append(&child);

    assert_parent_conflict(result, "parent", ParentConflictReason::DifferentWorkdayDate);
    assert_eq!(read_reopened(&database, &child_workday), []);
    assert_eq!(read_reopened(&database, &workday()), [parent]);
}

#[test]
fn different_timezone_cannot_create_an_orphaned_child_in_a_reopened_workday_read() {
    let database = TemporaryDatabase::new();
    let repository = ManualAdjustmentRepository::new(&database.connection);
    let parent = duration("parent", workday(), 30, at(12, 0));
    repository.append(&parent).unwrap();
    let child_workday = LocalWorkday::new(workday().date, "UTC");
    let child = duration("child", child_workday.clone(), 40, at(12, 1)).superseding(id("parent"));

    let result = repository.append(&child);

    assert_parent_conflict(result, "parent", ParentConflictReason::DifferentTimezone);
    assert_eq!(read_reopened(&database, &child_workday), []);
    assert_eq!(read_reopened(&database, &workday()), [parent]);
}

#[test]
fn successor_created_at_must_be_strictly_after_parent() {
    for (hour, minute) in [(11, 59), (12, 0)] {
        let database = TemporaryDatabase::new();
        let repository = ManualAdjustmentRepository::new(&database.connection);
        repository
            .append(&duration("parent", workday(), 30, at(12, 0)))
            .unwrap();
        let child = duration("child", workday(), 40, at(hour, minute)).superseding(id("parent"));

        let result = repository.append(&child);

        assert_parent_conflict(
            result,
            "parent",
            ParentConflictReason::NonIncreasingCreatedAt,
        );
        assert_eq!(database.count("ManualAdjustment"), 1);
    }
}

/// Appends each adjustment from its own thread and connection, all at once; with `current`,
/// through the validated append.
fn append_concurrently(
    path: &Path,
    current: Option<&AccountingResult>,
    adjustments: &[ManualAdjustment],
) -> Vec<Result<AppendAdjustmentResult, ManualAdjustmentError>> {
    let start = Barrier::new(adjustments.len());
    std::thread::scope(|scope| {
        let workers: Vec<_> = adjustments
            .iter()
            .map(|adjustment| {
                let start = &start;
                scope.spawn(move || {
                    let connection = open(path).unwrap();
                    let repository = ManualAdjustmentRepository::new(&connection);
                    start.wait();
                    match current {
                        Some(current) => repository.append_validated(current, adjustment),
                        None => repository.append(adjustment),
                    }
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
fn concurrent_exact_retries_converge_on_one_row() {
    let database = TemporaryDatabase::new();
    let adjustment = interval_work("concurrent", between(7, 8), at(12, 0));

    let results = append_concurrently(&database.path(), None, &[adjustment.clone(), adjustment]);

    let mut outcomes: Vec<_> = results
        .into_iter()
        .map(|result| result.unwrap().outcome)
        .collect();
    outcomes.sort_by_key(|outcome| *outcome == AppendAdjustmentOutcome::AlreadyRecorded);
    assert_eq!(
        outcomes,
        [
            AppendAdjustmentOutcome::Inserted,
            AppendAdjustmentOutcome::AlreadyRecorded
        ]
    );
    assert_eq!(database.count("ManualAdjustment"), 1);
}

#[test]
fn concurrent_competing_successors_expose_one_typed_conflict() {
    let database = TemporaryDatabase::new();
    ManualAdjustmentRepository::new(&database.connection)
        .append(&duration("root", workday(), 30, at(12, 0)))
        .unwrap();
    let successors = [
        duration("successor-a", workday(), 40, at(12, 1)).superseding(id("root")),
        duration("successor-b", workday(), 50, at(12, 2)).superseding(id("root")),
    ];

    let (succeeded, failed): (Vec<_>, Vec<_>) =
        append_concurrently(&database.path(), None, &successors)
            .into_iter()
            .partition(Result::is_ok);

    assert_eq!(succeeded.len(), 1);
    let [
        Err(ManualAdjustmentError::SupersessionConflict {
            superseded_id,
            existing_successor_id,
        }),
    ] = failed.as_slice()
    else {
        panic!("{failed:?}");
    };
    assert_eq!(*superseded_id, id("root"));
    assert!([id("successor-a"), id("successor-b")].contains(existing_successor_id));
    assert_eq!(database.count("ManualAdjustment"), 2);
}

#[test]
fn concurrent_validated_overlapping_roots_allow_only_one_renderable_adjustment() {
    let database = TemporaryDatabase::new();
    let current = account(Vec::new());
    let roots = [
        exclusion("root-a", between(8, 10), at(12, 0)),
        exclusion("root-b", between(9, 11), at(12, 1)),
    ];

    let (succeeded, failed): (Vec<_>, Vec<_>) =
        append_concurrently(&database.path(), Some(&current), &roots)
            .into_iter()
            .partition(Result::is_ok);

    assert_eq!(succeeded.len(), 1);
    assert!(
        matches!(
            failed.as_slice(),
            [Err(ManualAdjustmentError::Validation(
                AdjustmentValidationError::OverlappingActiveAdjustments
            ))]
        ),
        "{failed:?}"
    );
    let stored = ManualAdjustmentRepository::new(&database.connection)
        .read_for_workday(&workday())
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(
        account(stored).timeline().excluded_work(),
        SignedDuration::from_hours(2)
    );
}

#[test]
fn validated_exact_retry_returns_the_existing_adjustment() {
    let database = TemporaryDatabase::new();
    let repository = ManualAdjustmentRepository::new(&database.connection);
    let current = account(Vec::new());
    let adjustment = duration("validated-retry", workday(), 30, at(12, 0));

    let first = repository.append_validated(&current, &adjustment).unwrap();
    let retry = repository.append_validated(&current, &adjustment).unwrap();

    assert_eq!(first.outcome, AppendAdjustmentOutcome::Inserted);
    assert_eq!(retry.outcome, AppendAdjustmentOutcome::AlreadyRecorded);
    assert_eq!(first.authoritative_id, retry.authoritative_id);
    assert_eq!(database.count("ManualAdjustment"), 1);
}

#[test]
fn migrated_v1_adjustment_uses_requested_timezone_after_reopen_and_remains_accountable() {
    let database = TemporaryDatabase::at_version(1);
    database
        .connection
        .execute(
            "INSERT INTO ManualAdjustment (
    Id,
    WorkdayDate,
    Kind,
    DurationMinutes,
    Reason,
    CreatedAtUtc)
VALUES (
    $id,
    $workdayDate,
    $kind,
    $durationMinutes,
    $reason,
    $createdAtUtc);",
            named_params! {
                "$id": "legacy-duration",
                "$workdayDate": "2026-08-28",
                "$kind": "add-work",
                "$durationMinutes": 30,
                "$reason": "Imported V1 adjustment",
                "$createdAtUtc": "2026-08-28T12:00:00.0000000+00:00",
            },
        )
        .unwrap();
    database.migrate();

    let restored = read_reopened(&database, &workday());

    let [restored] = restored.as_slice() else {
        panic!("{restored:?}");
    };
    assert_eq!(*restored.id(), id("legacy-duration"));
    assert_eq!(*restored.workday(), workday());
    assert_eq!(
        *restored.kind(),
        AdjustmentKind::DurationWork {
            duration: SignedDuration::from_mins(30)
        }
    );
    assert_eq!(restored.reason(), Some("Imported V1 adjustment"));
    let accounting = account(vec![restored.clone()]);
    assert_eq!(accounting.credited_work(), SignedDuration::from_mins(30));
    assert_eq!(accounting.manual_duration_credits().len(), 1);
}

#[test]
fn read_filters_by_date_and_timezone_and_orders_by_created_at_then_id() {
    let database = TemporaryDatabase::new();
    let repository = ManualAdjustmentRepository::new(&database.connection);
    repository
        .append(&duration("z-last", workday(), 5, at(12, 0)))
        .unwrap();
    repository
        .append(&duration("a-first", workday(), 10, at(12, 0)))
        .unwrap();
    repository
        .append(&duration(
            "other-zone",
            LocalWorkday::new(workday().date, "UTC"),
            15,
            at(12, 1),
        ))
        .unwrap();

    let restored = repository.read_for_workday(&workday()).unwrap();

    let ids: Vec<_> = restored
        .iter()
        .map(|adjustment| adjustment.id().clone())
        .collect();
    assert_eq!(ids, [id("a-first"), id("z-last")]);
}

#[test]
fn a_duration_in_fractional_minutes_is_refused_before_writing() {
    let database = TemporaryDatabase::new();
    let adjustment = ManualAdjustment::duration_work(
        id("fractional"),
        workday(),
        SignedDuration::from_secs(90),
        at(12, 0),
    )
    .unwrap();

    let result = ManualAdjustmentRepository::new(&database.connection).append(&adjustment);

    assert!(
        matches!(&result, Err(ManualAdjustmentError::FractionalMinutes(fractional)) if *fractional == id("fractional")),
        "{result:?}"
    );
    assert_eq!(database.count("ManualAdjustment"), 0);
}

#[test]
fn reading_rejects_a_stored_row_that_forms_no_adjustment() {
    let database = TemporaryDatabase::new();
    database
        .connection
        .execute_batch(
            "INSERT INTO ManualAdjustment (
    Id, WorkdayDate, Kind, StartUtc, EndUtc, CreatedAtUtc, TimezoneId)
VALUES (
    'backwards', '2026-08-28', 'exclude-time', '2026-08-28T09:00:00.0000000+00:00',
    '2026-08-28T08:00:00.0000000+00:00', '2026-08-28T12:00:00.0000000+00:00',
    'Romance Standard Time');",
        )
        .unwrap();

    let result = ManualAdjustmentRepository::new(&database.connection).read_for_workday(&workday());

    assert!(
        matches!(
            &result,
            Err(ManualAdjustmentError::Storage(StorageError::InvalidStoredValue(invalid)))
                if invalid.detail == "the interval must have a positive duration"
        ),
        "{result:?}"
    );
}

#[test]
fn a_validated_retry_with_sub_tick_instants_returns_the_existing_adjustment() {
    let database = TemporaryDatabase::new();
    let repository = ManualAdjustmentRepository::new(&database.connection);
    let current = account(Vec::new());
    let nanosecond = SignedDuration::from_nanos(1);
    let created_at = at(12, 0) + nanosecond;
    let retries = [
        duration("sub-tick-duration", workday(), 30, created_at),
        interval_work(
            "sub-tick-interval",
            UtcInterval::new(at(9, 0) + nanosecond, at(10, 0) + nanosecond).unwrap(),
            created_at,
        ),
    ];

    for adjustment in &retries {
        let first = repository.append_validated(&current, adjustment).unwrap();
        let retry = repository.append_validated(&current, adjustment).unwrap();

        assert_eq!(first.outcome, AppendAdjustmentOutcome::Inserted);
        assert_eq!(retry.outcome, AppendAdjustmentOutcome::AlreadyRecorded);
    }
    assert_eq!(database.count("ManualAdjustment"), 2);
}

#[test]
fn an_interval_shorter_than_a_tick_is_refused_before_writing() {
    let database = TemporaryDatabase::new();
    let repository = ManualAdjustmentRepository::new(&database.connection);
    let start = at(9, 0) + SignedDuration::from_nanos(10);
    let adjustment = interval_work(
        "sub-tick-only",
        UtcInterval::new(start, start + SignedDuration::from_nanos(50)).unwrap(),
        at(12, 0),
    );

    let result = repository.append(&adjustment);

    assert!(
        matches!(result, Err(ManualAdjustmentError::IntervalShorterThanATick(ref id)) if id.as_str() == "sub-tick-only"),
        "{result:?}"
    );
    assert_eq!(database.count("ManualAdjustment"), 0);
}

#[test]
fn a_batch_refused_at_its_second_adjustment_records_none() {
    let database = TemporaryDatabase::new();
    let repository = ManualAdjustmentRepository::new(&database.connection);
    let current = account(Vec::new());
    let batch = [
        duration("first", workday(), 30, at(12, 0)),
        // The id of the first, for another adjustment.
        duration("first", workday(), 45, at(12, 0)),
        duration("third", workday(), 15, at(12, 0)),
    ];

    let refused = repository.append_all_validated(&current, &batch);

    assert!(
        matches!(refused, Err(ManualAdjustmentError::Validation(_))),
        "{refused:?}"
    );
    assert_eq!(database.count("ManualAdjustment"), 0);
}
