use std::collections::HashSet;
use std::sync::Barrier;

use bientot_domain::accounting::{
    AccountingInput, AccountingLabel, AccountingPolicy, AccountingResult, ActivityCoverage,
    ActivitySegment, ActivityState, SegmentKind, calculate,
};
use bientot_domain::intervals::UtcInterval;
use bientot_domain::time::{FrozenClock, LocalWorkday};
use jiff::civil::date;
use jiff::{SignedDuration, Timestamp};

use super::*;
use crate::database::open;
use crate::test_support::TemporaryDatabase;

const ZONE: &str = "Romance Standard Time";

fn created_at() -> Timestamp {
    at(12, 0)
}

fn at(hour: i8, minute: i8) -> Timestamp {
    date(2026, 8, 30)
        .at(hour, minute, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn key(value: &str) -> OperationKey {
    OperationKey::new(value).unwrap()
}

fn session(at: Timestamp, state: SessionState) -> WorkdayTransition {
    WorkdayTransition::new(at, ZONE, WorkdayTransitionKind::SessionObserved(state)).unwrap()
}

fn power(at: Timestamp, state: PowerState) -> WorkdayTransition {
    WorkdayTransition::new(at, ZONE, WorkdayTransitionKind::PowerObserved(state)).unwrap()
}

fn tracking(at: Timestamp, state: TrackingState) -> WorkdayTransition {
    WorkdayTransition::new(at, ZONE, WorkdayTransitionKind::TrackingChanged(state)).unwrap()
}

fn atomic(operation_key: &str, transition: WorkdayTransition) -> AtomicWorkdayTransition {
    AtomicWorkdayTransition {
        operation_key: key(operation_key),
        transition,
    }
}

fn window_between(start_hour: i8, end_hour: i8) -> WorkdayWindow {
    WorkdayWindow {
        workday: LocalWorkday::new(date(2026, 8, 30), ZONE),
        start: at(start_hour, 0),
        end: at(end_hour, 0),
    }
}

fn window() -> WorkdayWindow {
    window_between(7, 17)
}

fn short_window() -> WorkdayWindow {
    window_between(7, 8)
}

/// Accounts up to the end of the read window while the workday itself is still live.
fn account(window: &WorkdayWindow, history: WorkdayEventHistory) -> AccountingResult {
    let bounds = UtcInterval::new(window.start, window.end).unwrap();
    let activity = ActivityCoverage::new(
        bounds,
        [ActivitySegment::new(
            bounds,
            ActivityState::Active,
            AutomaticOrigin::new("activitywatch").unwrap(),
        )],
    );
    let policy = AccountingPolicy::new(
        UtcInterval::new(at(12, 0), at(13, 0)).unwrap(),
        SignedDuration::from_mins(90),
    )
    .unwrap();
    let live_workday = WorkdayWindow {
        end: window.end + SignedDuration::from_hours(1),
        ..window.clone()
    };
    calculate(
        &AccountingInput::new(
            live_workday,
            window.end,
            policy,
            activity,
            history,
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn ids(results: &[RecordTransitionResult]) -> Vec<WorkdayEventId> {
    results
        .iter()
        .map(|result| result.authoritative_id)
        .collect()
}

fn in_window_ids(history: &WorkdayEventHistory) -> Vec<WorkdayEventId> {
    history
        .in_window()
        .iter()
        .map(RecordedTransition::id)
        .collect()
}

#[test]
fn atomic_power_interval_retry_preserves_the_same_rows() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);
    let interval = [
        atomic(
            "history:power:unknown:record-80:tick-1",
            power(at(8, 0), PowerState::Unknown),
        ),
        atomic(
            "history:power:suspended:record-80:tick-1",
            power(at(8, 0), PowerState::Suspended),
        ),
        atomic(
            "history:power:active:record-81:tick-2",
            power(at(9, 0), PowerState::Active),
        ),
    ];

    let first = repository.record_atomically(&interval).unwrap();
    let reopened = database.reopen();
    let retry = SessionEventRepository::new(&reopened, &clock)
        .record_atomically(&interval)
        .unwrap();

    assert!(
        first
            .iter()
            .all(|result| result.outcome != RecordTransitionOutcome::AlreadyRecorded)
    );
    assert!(
        retry
            .iter()
            .all(|result| result.outcome == RecordTransitionOutcome::AlreadyRecorded)
    );
    assert_eq!(ids(&first), ids(&retry));
    assert_eq!(database.count("SessionEvent"), 3);
    let history = repository.read_for_accounting(&window()).unwrap();
    let states: Vec<_> = history
        .in_window()
        .iter()
        .filter_map(|item| match item {
            RecordedTransition::Power(recorded) => Some(recorded.state),
            _ => None,
        })
        .collect();
    assert_eq!(
        states,
        [
            PowerState::Unknown,
            PowerState::Suspended,
            PowerState::Active
        ]
    );
}

#[test]
fn atomic_power_interval_rolls_back_when_a_later_operation_key_conflicts() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);
    repository
        .record(
            key("history:power:end"),
            power(at(11, 0), PowerState::Active),
        )
        .unwrap();

    let result = repository.record_atomically(&[
        atomic(
            "history:power:unknown:start",
            power(at(8, 0), PowerState::Unknown),
        ),
        atomic(
            "history:power:suspended:start",
            power(at(8, 0), PowerState::Suspended),
        ),
        atomic("history:power:end", power(at(9, 0), PowerState::Active)),
    ]);

    assert!(
        matches!(&result, Err(SessionEventError::OperationConflict(conflict)) if *conflict == key("history:power:end")),
        "{result:?}"
    );
    assert_eq!(database.count("SessionEvent"), 1);
}

#[test]
fn known_state_at_same_instant_wins_when_it_becomes_the_next_window_anchor() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);
    let boundary = at(8, 0);
    let session_result = repository
        .record(
            key("real-session-at-boundary"),
            session(boundary, SessionState::Locked),
        )
        .unwrap();
    let power_result = repository
        .record(
            key("real-power-at-boundary"),
            power(boundary, PowerState::Suspended),
        )
        .unwrap();
    repository
        .record(
            key("later-session-reset-at-boundary"),
            session(boundary, SessionState::Unknown),
        )
        .unwrap();
    repository
        .record(
            key("later-power-reset-at-boundary"),
            power(boundary, PowerState::Unknown),
        )
        .unwrap();

    let next_window = WorkdayWindow {
        workday: LocalWorkday::new(date(2026, 8, 31), ZONE),
        start: at(7, 0) + SignedDuration::from_hours(24),
        end: at(17, 0) + SignedDuration::from_hours(24),
    };
    let history = repository.read_for_accounting(&next_window).unwrap();

    let anchors = history.before_window();
    let session_anchor = anchors.session.as_ref().unwrap();
    let power_anchor = anchors.power.as_ref().unwrap();
    assert_eq!(session_anchor.id, session_result.authoritative_id);
    assert_eq!(session_anchor.state, SessionState::Locked);
    assert_eq!(power_anchor.id, power_result.authoritative_id);
    assert_eq!(power_anchor.state, PowerState::Suspended);
}

#[test]
fn exact_retry_returns_the_existing_event_and_preserves_audit_fields() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);
    let operation_key = key("session-lock-callback-42");
    let transition = session(at(7, 0), SessionState::Locked);

    let first = repository
        .record(operation_key.clone(), transition.clone())
        .unwrap();
    clock.advance(SignedDuration::from_mins(5));
    let retry = repository.record(operation_key, transition).unwrap();

    assert_eq!(first.outcome, RecordTransitionOutcome::Inserted);
    assert_eq!(retry.outcome, RecordTransitionOutcome::AlreadyRecorded);
    assert_eq!(first.authoritative_id, retry.authoritative_id);
    let rows: Vec<(String, String, String, String)> = database
        .connection
        .prepare("SELECT OperationKey, TimezoneId, Source, CreatedAtUtc FROM SessionEvent;")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        [(
            "session-lock-callback-42".to_owned(),
            ZONE.to_owned(),
            "windows-session".to_owned(),
            "2026-08-30T12:00:00.0000000+00:00".to_owned(),
        )]
    );
}

#[test]
fn same_operation_key_with_different_payload_is_a_conflict() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);
    let operation_key = key("power-callback-42");
    repository
        .record(
            operation_key.clone(),
            power(at(7, 0), PowerState::Suspended),
        )
        .unwrap();

    let result = repository.record(
        operation_key.clone(),
        power(at(7, 1), PowerState::Suspended),
    );

    assert!(
        matches!(&result, Err(SessionEventError::OperationConflict(conflict)) if *conflict == operation_key),
        "{result:?}"
    );
    assert_eq!(database.count("SessionEvent"), 1);
}

#[test]
fn same_state_in_the_same_lane_persists_a_no_state_change_receipt() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);
    let first = repository
        .record(
            key("session-probe-1"),
            session(at(7, 0), SessionState::Unlocked),
        )
        .unwrap();

    let second = repository
        .record(
            key("session-probe-2"),
            session(at(7, 1), SessionState::Unlocked),
        )
        .unwrap();

    assert_eq!(second.outcome, RecordTransitionOutcome::NoStateChange);
    assert_ne!(first.authoritative_id, second.authoritative_id);
    assert_eq!(database.count("SessionEvent"), 2);
}

#[test]
fn equal_looking_states_in_different_lanes_remain_independent() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);

    let results = [
        repository.record(key("session-lock"), session(at(7, 0), SessionState::Locked)),
        repository.record(key("power-suspend"), power(at(7, 0), PowerState::Suspended)),
        repository.record(
            key("tracking-pause"),
            tracking(at(7, 0), TrackingState::Paused),
        ),
    ];

    assert!(
        results
            .iter()
            .all(|result| result.as_ref().unwrap().outcome == RecordTransitionOutcome::Inserted)
    );
    assert_eq!(database.count("SessionEvent"), 3);
}

#[test]
fn a_session_read_from_the_windows_logs_keeps_its_lane_and_names_its_origin() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);
    repository
        .record(
            key(&format!("{SESSION_HISTORY_KEY_PREFIX}unlocked:1")),
            session(at(8, 0), SessionState::Unlocked),
        )
        .unwrap();
    repository
        .record(key("session-lock"), session(at(9, 0), SessionState::Locked))
        .unwrap();

    let history = repository.read_for_accounting(&window()).unwrap();

    let origins: Vec<_> = history
        .in_window()
        .iter()
        .map(|transition| match transition {
            RecordedTransition::Session(session) => {
                (session.state, session.origin.source_id().to_owned())
            }
            other => panic!("expected a session transition, got {other:?}"),
        })
        .collect();
    assert_eq!(
        origins,
        [
            (SessionState::Unlocked, SESSION_HISTORY_SOURCE_ID.to_owned()),
            (SessionState::Locked, "windows-session".to_owned()),
        ]
    );
}

/// Records each `(operation key, transition)` from its own thread and connection, all at once.
fn record_concurrently(
    database: &TemporaryDatabase,
    attempts: &[(&str, WorkdayTransition)],
) -> Vec<RecordTransitionResult> {
    let path = database.path();
    let start = Barrier::new(attempts.len());
    std::thread::scope(|scope| {
        let workers: Vec<_> = attempts
            .iter()
            .map(|(operation_key, transition)| {
                let (path, start) = (&path, &start);
                scope.spawn(move || {
                    let connection = open(path).unwrap();
                    let clock = FrozenClock::new(created_at());
                    let repository = SessionEventRepository::new(&connection, &clock);
                    start.wait();
                    repository
                        .record(key(operation_key), transition.clone())
                        .unwrap()
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
fn concurrent_retry_with_the_same_operation_key_converges_on_one_row() {
    let database = TemporaryDatabase::new();
    let transition = power(at(7, 0), PowerState::Active);

    let results = record_concurrently(
        &database,
        &[
            ("duplicate-resume-callback", transition.clone()),
            ("duplicate-resume-callback", transition),
        ],
    );

    let outcomes: HashSet<_> = results.iter().map(|result| result.outcome).collect();
    assert_eq!(
        outcomes,
        HashSet::from([
            RecordTransitionOutcome::Inserted,
            RecordTransitionOutcome::AlreadyRecorded
        ])
    );
    assert_eq!(results[0].authoritative_id, results[1].authoritative_id);
    assert_eq!(database.count("SessionEvent"), 1);
}

#[test]
fn concurrent_duplicate_callbacks_with_different_keys_converge_by_lane_state() {
    let database = TemporaryDatabase::new();
    let transition = session(at(7, 0), SessionState::Locked);

    let results = record_concurrently(
        &database,
        &[
            ("duplicate-lock-1", transition.clone()),
            ("duplicate-lock-2", transition),
        ],
    );

    let outcomes: HashSet<_> = results.iter().map(|result| result.outcome).collect();
    assert_eq!(
        outcomes,
        HashSet::from([
            RecordTransitionOutcome::Inserted,
            RecordTransitionOutcome::NoStateChange
        ])
    );
    assert_ne!(results[0].authoritative_id, results[1].authoritative_id);
    assert_eq!(database.count("SessionEvent"), 2);

    let clock = FrozenClock::new(created_at());
    let history = SessionEventRepository::new(&database.connection, &clock)
        .read_for_accounting(&short_window())
        .unwrap();
    let accounting = account(&short_window(), history);
    let [segment] = accounting.timeline().segments() else {
        panic!("{:?}", accounting.timeline().segments());
    };
    assert!(matches!(
        segment.kind(),
        SegmentKind::ProvisionalCredit { .. }
    ));
    assert_eq!(
        segment.interval(),
        UtcInterval::new(short_window().start, short_window().end).unwrap()
    );
}

#[test]
fn delayed_transition_uses_its_occurrence_time_predecessor_and_changes_accounting_from_that_instant()
 {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);
    let initial = repository
        .record(
            key("session-unlocked-at-seven"),
            session(at(7, 0), SessionState::Unlocked),
        )
        .unwrap();
    let later = repository
        .record(
            key("session-locked-at-ten"),
            session(at(10, 0), SessionState::Locked),
        )
        .unwrap();

    let delayed = repository
        .record(
            key("session-locked-at-nine"),
            session(at(9, 0), SessionState::Locked),
        )
        .unwrap();

    assert_eq!(delayed.outcome, RecordTransitionOutcome::Inserted);
    let history = repository
        .read_for_accounting(&window_between(7, 11))
        .unwrap();
    assert_eq!(
        in_window_ids(&history),
        [
            initial.authoritative_id,
            delayed.authoritative_id,
            later.authoritative_id
        ]
    );
    let accounting = account(&window_between(7, 11), history);
    let provisional: Vec<_> = accounting
        .timeline()
        .segments()
        .iter()
        .filter(|segment| matches!(segment.kind(), SegmentKind::ProvisionalCredit { .. }))
        .collect();
    let [locked] = provisional.as_slice() else {
        panic!("{provisional:?}");
    };
    assert_eq!(locked.interval().start(), at(9, 0));
}

#[test]
fn no_state_change_receipt_survives_an_opposite_event_and_exact_retry() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);
    repository
        .record(
            key("initial-unlocked"),
            session(at(7, 0), SessionState::Unlocked),
        )
        .unwrap();
    let transition = session(at(8, 0), SessionState::Unlocked);
    let no_change = repository
        .record(key("redundant-unlocked"), transition.clone())
        .unwrap();
    repository
        .record(key("later-locked"), session(at(9, 0), SessionState::Locked))
        .unwrap();

    let retry = repository
        .record(key("redundant-unlocked"), transition)
        .unwrap();

    assert_eq!(no_change.outcome, RecordTransitionOutcome::NoStateChange);
    assert_eq!(retry.outcome, RecordTransitionOutcome::AlreadyRecorded);
    assert_eq!(no_change.authoritative_id, retry.authoritative_id);
    assert_eq!(database.count("SessionEvent"), 3);
}

#[test]
fn same_key_with_different_payload_conflicts_after_a_no_state_change_receipt() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);
    repository
        .record(
            key("initial-unlocked"),
            session(at(7, 0), SessionState::Unlocked),
        )
        .unwrap();
    let no_change = repository
        .record(
            key("redundant-unlocked"),
            session(at(8, 0), SessionState::Unlocked),
        )
        .unwrap();

    let result = repository.record(
        key("redundant-unlocked"),
        session(at(8, 1), SessionState::Unlocked),
    );

    assert_eq!(no_change.outcome, RecordTransitionOutcome::NoStateChange);
    assert!(
        matches!(&result, Err(SessionEventError::OperationConflict(conflict)) if *conflict == key("redundant-unlocked")),
        "{result:?}"
    );
    assert_eq!(database.count("SessionEvent"), 2);
}

#[test]
fn database_reopen_preserves_a_no_state_change_receipt() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);
    repository
        .record(
            key("initial-unlocked"),
            session(at(7, 0), SessionState::Unlocked),
        )
        .unwrap();
    let transition = session(at(8, 0), SessionState::Unlocked);
    let no_change = repository
        .record(key("redundant-unlocked"), transition.clone())
        .unwrap();

    let reopened = database.reopen();
    let retry = SessionEventRepository::new(&reopened, &clock)
        .record(key("redundant-unlocked"), transition)
        .unwrap();

    assert_eq!(no_change.outcome, RecordTransitionOutcome::NoStateChange);
    assert_eq!(retry.outcome, RecordTransitionOutcome::AlreadyRecorded);
    assert_eq!(no_change.authoritative_id, retry.authoritative_id);
    assert_eq!(database.count("SessionEvent"), 2);
}

#[test]
fn pre_window_anchor_keeps_the_first_observation_in_the_current_state_run() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);
    let first_lock = repository
        .record(
            key("lock-at-seven"),
            session(at(7, 0), SessionState::Locked),
        )
        .unwrap();
    let redundant_lock = repository
        .record(
            key("lock-at-eight"),
            session(at(8, 0), SessionState::Locked),
        )
        .unwrap();

    let first_history = repository
        .read_for_accounting(&window_between(9, 10))
        .unwrap();

    assert_eq!(
        first_history.before_window().session.as_ref().unwrap().id,
        first_lock.authoritative_id
    );
    let accounting = account(&window_between(9, 10), first_history);
    let [segment] = accounting.timeline().segments() else {
        panic!("{:?}", accounting.timeline().segments());
    };
    assert_eq!(segment.label(), AccountingLabel::OvernightLock);

    repository
        .record(
            key("delayed-unlock-at-seven-thirty"),
            session(at(7, 30), SessionState::Unlocked),
        )
        .unwrap();
    let corrected_history = repository
        .read_for_accounting(&window_between(9, 10))
        .unwrap();

    assert_eq!(
        corrected_history
            .before_window()
            .session
            .as_ref()
            .unwrap()
            .id,
        redundant_lock.authoritative_id
    );
}

#[test]
fn accounting_read_returns_one_pre_window_anchor_per_lane() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);
    repository
        .record(
            key("old-session-lock"),
            session(at(5, 0), SessionState::Locked),
        )
        .unwrap();
    let session_anchor = repository
        .record(
            key("session-unlock"),
            session(at(6, 0), SessionState::Unlocked),
        )
        .unwrap();
    let power_anchor = repository
        .record(
            key("power-suspend"),
            power(at(6, 30), PowerState::Suspended),
        )
        .unwrap();
    let tracking_anchor = repository
        .record(
            key("tracking-pause"),
            tracking(at(6, 45), TrackingState::Paused),
        )
        .unwrap();
    let in_window = repository
        .record(
            key("session-relock"),
            session(at(8, 0), SessionState::Locked),
        )
        .unwrap();

    let history = repository.read_for_accounting(&window()).unwrap();

    let anchors = history.before_window();
    assert_eq!(
        anchors.session.as_ref().unwrap().id,
        session_anchor.authoritative_id
    );
    assert_eq!(
        anchors.power.as_ref().unwrap().id,
        power_anchor.authoritative_id
    );
    assert_eq!(
        anchors.tracking.as_ref().unwrap().id,
        tracking_anchor.authoritative_id
    );
    assert_eq!(in_window_ids(&history), [in_window.authoritative_id]);
}

#[test]
fn accounting_read_orders_transitions_by_occurrence_then_id() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);
    let tied_first = repository
        .record(key("power-at-ten"), power(at(10, 0), PowerState::Suspended))
        .unwrap();
    let tied_second = repository
        .record(
            key("session-at-ten"),
            session(at(10, 0), SessionState::Locked),
        )
        .unwrap();
    let earlier = repository
        .record(
            key("tracking-at-nine"),
            tracking(at(9, 0), TrackingState::Paused),
        )
        .unwrap();

    let history = repository.read_for_accounting(&window()).unwrap();

    assert_eq!(
        in_window_ids(&history),
        [
            earlier.authoritative_id,
            tied_first.authoritative_id,
            tied_second.authoritative_id
        ]
    );
}

#[test]
fn accounting_read_rejects_malformed_stored_rows() {
    let database = TemporaryDatabase::new();
    database
        .connection
        .execute_batch(
            "INSERT INTO SessionEvent (
    OccurredAtUtc,
    TimezoneId,
    Kind,
    Source,
    CreatedAtUtc,
    OperationKey)
VALUES (
    '2026-08-30T09:00:00.0000000+00:00',
    'Romance Standard Time',
    'locked',
    'windows-session',
    'not-an-instant',
    'malformed-row');",
        )
        .unwrap();
    let clock = FrozenClock::new(created_at());

    let result =
        SessionEventRepository::new(&database.connection, &clock).read_for_accounting(&window());

    assert!(
        matches!(
            &result,
            Err(SessionEventError::Storage(StorageError::InvalidStoredValue(invalid)))
                if invalid.detail == "CreatedAtUtc: 'not-an-instant' is not a .NET round-trip UTC instant"
        ),
        "{result:?}"
    );
}

#[test]
fn a_batch_naming_one_operation_key_twice_records_nothing() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);

    let result = repository.record_atomically(&[
        atomic("twice", power(at(8, 0), PowerState::Suspended)),
        atomic("twice", power(at(9, 0), PowerState::Active)),
    ]);

    assert!(
        matches!(&result, Err(SessionEventError::DuplicateOperationKey(duplicate)) if *duplicate == key("twice")),
        "{result:?}"
    );
    assert_eq!(database.count("SessionEvent"), 0);
}

#[test]
fn an_instant_finer_than_a_tick_is_stored_truncated_and_retries_as_the_same_transition() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(created_at());
    let repository = SessionEventRepository::new(&database.connection, &clock);
    let fine = at(8, 0) + SignedDuration::from_nanos(1_234_567_891);

    let first = repository
        .record(key("fine"), session(fine, SessionState::Locked))
        .unwrap();
    let retry = repository
        .record(key("fine"), session(fine, SessionState::Locked))
        .unwrap();

    assert_eq!(retry.outcome, RecordTransitionOutcome::AlreadyRecorded);
    assert_eq!(retry.authoritative_id, first.authoritative_id);
    let stored: String = database
        .connection
        .query_row("SELECT OccurredAtUtc FROM SessionEvent;", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(stored, "2026-08-30T08:00:01.2345678+00:00");
}
