//! Ported from `TrackingControlTests.cs` and `ReopenedTrayUsesPersistedTrackingState` of
//! `TrackingControlIntegrationTests.cs`, against a migrated SQLite ledger instead of the .NET
//! in-memory store.

use std::cell::Cell;
use std::collections::HashSet;
use std::sync::Barrier;

use jiff::{SignedDuration, Timestamp};
use rusqlite::Connection;
use worktime_domain::accounting::{
    AccountingInput, AccountingLabel, AccountingPolicy, ActivityCoverage, ActivitySegment,
    ActivityState, AutomaticOrigin, calculate,
};
use worktime_domain::intervals::UtcInterval;
use worktime_domain::time::{FixedTimeZoneProvider, FrozenClock, WorkdayWindow};
use worktime_store::codec::parse_instant;

use super::*;
use crate::test_support::{Database, utc, utc_zones};

fn workday(month: i8, day: i8) -> LocalWorkday {
    LocalWorkday::new(jiff::civil::date(2026, month, day), "UTC")
}

fn monday() -> LocalWorkday {
    workday(8, 24)
}

fn at(workday: &LocalWorkday, hour: i8) -> Timestamp {
    utc(
        workday.date.year(),
        workday.date.month(),
        workday.date.day(),
        hour,
    )
}

fn window(workday: &LocalWorkday) -> WorkdayWindow {
    WorkdayCalendar::new(utc_zones()).window(workday).unwrap()
}

fn key(value: &str) -> OperationKey {
    OperationKey::new(value).unwrap()
}

/// A UTC tracking service on `connection`, whose refresh requests are ignored.
fn service<'a>(
    connection: &'a Connection,
    clock: &'a FrozenClock,
    zones: &'a FixedTimeZoneProvider,
) -> TrackingControl<'a> {
    TrackingControl::new(
        SessionEventRepository::new(connection, clock),
        clock,
        zones,
        &ignore_refresh,
    )
}

fn ignore_refresh() {}

#[derive(Debug, PartialEq, Eq)]
struct StoredTracking {
    operation_key: String,
    occurred_at: Timestamp,
    time_zone_id: String,
    kind: String,
}

/// The user-lane rows of the ledger, in insertion order.
fn tracking_rows(connection: &Connection) -> Vec<StoredTracking> {
    connection
        .prepare(
            "SELECT OperationKey, OccurredAtUtc, TimezoneId, Kind FROM SessionEvent \
             WHERE Source = 'user' ORDER BY Id",
        )
        .unwrap()
        .query_map([], |row| {
            Ok(StoredTracking {
                operation_key: row.get(0)?,
                occurred_at: parse_instant(&row.get::<_, String>(1)?).unwrap(),
                time_zone_id: row.get(2)?,
                kind: row.get(3)?,
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn resets(connection: &Connection) -> Vec<StoredTracking> {
    tracking_rows(connection)
        .into_iter()
        .filter(|row| row.kind == "tracking-resumed")
        .collect()
}

/// The tracking state of `workday` at `through_hour`, as the ledger tells it.
fn effective_state(
    connection: &Connection,
    workday: &LocalWorkday,
    through_hour: i8,
) -> TrackingState {
    let clock = FrozenClock::new(Timestamp::UNIX_EPOCH);
    let history = SessionEventRepository::new(connection, &clock)
        .read_for_accounting(&window(workday))
        .unwrap();
    let through = at(workday, through_hour);
    history
        .in_window()
        .iter()
        .rev()
        .find_map(|transition| match transition {
            RecordedTransition::Tracking(recorded) if recorded.occurred_at <= through => {
                Some(recorded.state)
            }
            _ => None,
        })
        .or(history
            .before_window()
            .tracking
            .as_ref()
            .map(|anchor| anchor.state))
        .unwrap_or(TrackingState::Running)
}

#[test]
fn pause_persists_across_later_workdays_until_explicit_resume() {
    let database = Database::new();
    let clock = FrozenClock::new(at(&monday(), 10));
    let zones = utc_zones();
    let tracking = service(&database.connection, &clock, &zones);
    tracking.pause(key("pause-monday")).unwrap();
    let next_monday = workday(8, 31);

    assert_eq!(tracking.reconcile(&workday(8, 25)).unwrap(), None);
    assert_eq!(tracking.reconcile(&next_monday).unwrap(), None);
    assert_eq!(
        effective_state(&database.connection, &next_monday, 0),
        TrackingState::Paused
    );

    clock.set(at(&next_monday, 9));
    tracking.resume(key("resume-next-monday")).unwrap();

    assert_eq!(
        effective_state(&database.connection, &next_monday, 10),
        TrackingState::Running
    );
    assert_eq!(tracking_rows(&database.connection).len(), 2);
}

#[test]
fn finish_excludes_the_rest_of_the_current_workday() {
    let database = Database::new();
    let clock = FrozenClock::new(at(&monday(), 10));
    let zones = utc_zones();
    service(&database.connection, &clock, &zones)
        .finish(key("finish-monday"))
        .unwrap();

    let window = window(&monday());
    let history = SessionEventRepository::new(&database.connection, &clock)
        .read_for_accounting(&window)
        .unwrap();
    let active = UtcInterval::new(window.start, at(&monday(), 17)).unwrap();
    let result = calculate(
        &AccountingInput::new(
            window,
            at(&monday(), 17),
            AccountingPolicy::new(
                UtcInterval::new(at(&monday(), 12), at(&monday(), 14)).unwrap(),
                SignedDuration::from_mins(90),
            )
            .unwrap(),
            ActivityCoverage::new(
                active,
                [ActivitySegment::new(
                    active,
                    ActivityState::Active,
                    AutomaticOrigin::new("activity").unwrap(),
                )],
            ),
            history,
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap();

    let finished: Vec<_> = result
        .timeline()
        .segments()
        .iter()
        .filter(|segment| segment.label() == AccountingLabel::DayFinished)
        .map(|segment| segment.interval())
        .collect();
    assert_eq!(
        finished,
        [UtcInterval::new(at(&monday(), 10), at(&monday(), 17)).unwrap()]
    );
}

#[test]
fn finished_resets_at_the_start_of_the_following_weekday() {
    let database = Database::new();
    let clock = FrozenClock::new(at(&monday(), 17));
    let zones = utc_zones();
    let tracking = service(&database.connection, &clock, &zones);
    tracking.finish(key("finish-monday")).unwrap();
    let tuesday = workday(8, 25);

    let reset = tracking.reconcile(&tuesday).unwrap();

    assert_eq!(
        reset.map(|reset| reset.outcome),
        Some(RecordTransitionOutcome::Inserted)
    );
    let last = tracking_rows(&database.connection).pop().unwrap();
    assert_eq!(last.kind, "tracking-resumed");
    assert_eq!(last.occurred_at, window(&tuesday).start);
    assert_eq!(last.time_zone_id, tuesday.time_zone_id);
}

#[test]
fn weekends_remain_inactive_and_never_reset_finished() {
    let database = Database::new();
    let clock = FrozenClock::new(at(&workday(8, 28), 17));
    let zones = utc_zones();
    let tracking = service(&database.connection, &clock, &zones);
    tracking.finish(key("finish-friday")).unwrap();

    assert_eq!(tracking.reconcile(&workday(8, 29)).unwrap(), None);
    assert_eq!(tracking.reconcile(&workday(8, 30)).unwrap(), None);

    let kinds: Vec<_> = tracking_rows(&database.connection)
        .into_iter()
        .map(|row| row.kind)
        .collect();
    assert_eq!(kinds, ["day-finished"]);
}

#[test]
fn friday_finish_resets_at_monday_boundary() {
    let database = Database::new();
    let clock = FrozenClock::new(at(&workday(8, 28), 17));
    let zones = utc_zones();
    let tracking = service(&database.connection, &clock, &zones);
    tracking.finish(key("finish-friday")).unwrap();
    let next_monday = workday(8, 31);

    tracking.reconcile(&next_monday).unwrap();

    let reset = tracking_rows(&database.connection).pop().unwrap();
    assert_eq!(reset.occurred_at, window(&next_monday).start);
    assert_eq!(reset.kind, "tracking-resumed");
}

#[test]
fn current_state_reconciliation_resets_friday_finish_when_monday_arrives_while_running() {
    let database = Database::new();
    let clock = FrozenClock::new(at(&workday(8, 28), 17));
    let zones = utc_zones();
    let tracking = service(&database.connection, &clock, &zones);
    tracking.finish(key("finish-friday-running")).unwrap();

    clock.set(at(&workday(8, 31), 9));
    let state = tracking.reconcile_current_state().unwrap();

    assert_eq!(state, TrackingState::Running);
    assert_eq!(resets(&database.connection).len(), 1);
}

#[test]
fn current_state_reconciliation_never_resets_pause_across_weekend() {
    let database = Database::new();
    let clock = FrozenClock::new(at(&workday(8, 28), 17));
    let zones = utc_zones();
    let tracking = service(&database.connection, &clock, &zones);
    tracking.pause(key("pause-friday-running")).unwrap();

    clock.set(at(&workday(8, 31), 9));
    let state = tracking.reconcile_current_state().unwrap();

    assert_eq!(state, TrackingState::Paused);
    assert_eq!(resets(&database.connection), []);
}

#[test]
fn late_startup_records_the_missed_next_workday_boundary() {
    let database = Database::new();
    let clock = FrozenClock::new(at(&workday(8, 27), 17));
    let zones = utc_zones();
    let tracking = service(&database.connection, &clock, &zones);
    tracking
        .finish(key("finish-before-offline-friday"))
        .unwrap();

    tracking.reconcile(&workday(8, 31)).unwrap();

    let reset = tracking_rows(&database.connection).pop().unwrap();
    assert_eq!(reset.occurred_at, window(&workday(8, 28)).start);
}

#[test]
fn restart_and_concurrent_retry_converge_on_one_deterministic_reset() {
    let database = Database::new();
    let tuesday = workday(8, 25);
    let zones = utc_zones();
    let finish_clock = FrozenClock::new(at(&monday(), 17));
    service(&database.connection, &finish_clock, &zones)
        .finish(key("finish-before-restart"))
        .unwrap();

    let start = Barrier::new(2);
    let outcomes: Vec<_> = std::thread::scope(|scope| {
        let retries: Vec<_> = (0..2)
            .map(|_| {
                let connection = database.reopen();
                let (start, tuesday) = (&start, &tuesday);
                scope.spawn(move || {
                    let clock = FrozenClock::new(at(tuesday, 8));
                    let zones = utc_zones();
                    let restarted = service(&connection, &clock, &zones);
                    start.wait();
                    restarted.reconcile(tuesday).unwrap().unwrap().outcome
                })
            })
            .collect();
        retries
            .into_iter()
            .map(|retry| retry.join().unwrap())
            .collect()
    });

    assert!(outcomes.contains(&RecordTransitionOutcome::Inserted));
    let resets = resets(&database.connection);
    assert_eq!(resets.len(), 1);
    let keys: HashSet<_> = resets.iter().map(|row| &row.operation_key).collect();
    assert_eq!(keys.len(), 1);
}

#[test]
fn explicit_command_retry_is_idempotent_and_operation_conflicts_are_visible() {
    let database = Database::new();
    let clock = FrozenClock::new(at(&monday(), 10));
    let zones = utc_zones();
    let tracking = service(&database.connection, &clock, &zones);
    let operation_key = key("explicit-command-retry");

    let first = tracking.pause(operation_key.clone()).unwrap();
    let retry = tracking.pause(operation_key.clone()).unwrap();
    let conflict = tracking.resume(operation_key.clone());

    assert_eq!(first.outcome, RecordTransitionOutcome::Inserted);
    assert_eq!(retry.outcome, RecordTransitionOutcome::AlreadyRecorded);
    assert!(matches!(
        conflict,
        Err(TrackingError::Events(SessionEventError::OperationConflict(conflicting)))
            if conflicting == operation_key
    ));
}

/// Not in the .NET tests: a retry stamps a later instant than the command it repeats.
#[test]
fn a_retry_after_time_advances_returns_the_original_result() {
    let database = Database::new();
    let clock = FrozenClock::new(at(&monday(), 10));
    let zones = utc_zones();
    let refreshes = Cell::new(0);
    let request_refresh = || refreshes.set(refreshes.get() + 1);
    let tracking = TrackingControl::new(
        SessionEventRepository::new(&database.connection, &clock),
        &clock,
        &zones,
        &request_refresh,
    );
    let operation_key = key("retried-pause");

    let first = tracking.pause(operation_key.clone()).unwrap();
    clock.set(at(&monday(), 11));
    let retry = tracking.pause(operation_key.clone()).unwrap();
    let connection = database.reopen();
    let reopened = service(&connection, &clock, &zones);
    let retry_after_restart = reopened.pause(operation_key.clone()).unwrap();
    let conflict = reopened.resume(operation_key.clone());

    for retried in [retry, retry_after_restart] {
        assert_eq!(retried.outcome, RecordTransitionOutcome::AlreadyRecorded);
        assert_eq!(retried.authoritative_id, first.authoritative_id);
    }
    assert_eq!(refreshes.get(), 1);
    assert_eq!(
        tracking_rows(&connection),
        [StoredTracking {
            operation_key: "retried-pause".into(),
            occurred_at: at(&monday(), 10),
            time_zone_id: "UTC".into(),
            kind: "tracking-paused".into(),
        }]
    );
    assert!(matches!(
        conflict,
        Err(TrackingError::Events(SessionEventError::OperationConflict(conflicting)))
            if conflicting == operation_key
    ));
}

#[test]
fn a_reopened_service_uses_the_persisted_tracking_state() {
    for persisted in [TrackingState::Paused, TrackingState::Finished] {
        let database = Database::new();
        let clock = FrozenClock::new(at(&monday(), 10));
        let zones = utc_zones();
        let first = service(&database.connection, &clock, &zones);
        match persisted {
            TrackingState::Paused => first.pause(key("persisted-pause")),
            _ => first.finish(key("persisted-finish")),
        }
        .unwrap();

        clock.set(at(&monday(), 11));
        let connection = database.reopen();
        let reopened = service(&connection, &clock, &zones);

        assert_eq!(reopened.reconcile_current_state().unwrap(), persisted);
    }
}

/// Not in the .NET tests: the refresh is asked for only when a command changed the ledger.
#[test]
fn only_a_command_that_changes_the_ledger_requests_a_refresh() {
    let database = Database::new();
    let clock = FrozenClock::new(at(&monday(), 10));
    let zones = utc_zones();
    let refreshes = Cell::new(0);
    let request_refresh = || refreshes.set(refreshes.get() + 1);
    let tracking = TrackingControl::new(
        SessionEventRepository::new(&database.connection, &clock),
        &clock,
        &zones,
        &request_refresh,
    );

    tracking.pause(key("pause")).unwrap();
    tracking.pause(key("pause")).unwrap();
    let paused_again = tracking.pause(key("pause-again")).unwrap();
    assert_eq!(refreshes.get(), 1);
    assert_eq!(paused_again.outcome, RecordTransitionOutcome::NoStateChange);

    tracking.resume(key("resume")).unwrap();
    assert_eq!(refreshes.get(), 2);
}
