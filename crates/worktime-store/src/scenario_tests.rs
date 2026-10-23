//! Replays the steps of the .NET fixture export's `RepositoryScenario` and compares what the
//! repositories return, what they read back and every row they write with what the .NET
//! repositories did, recorded in `tests/fixtures/repositories.json`.

use jiff::civil::{Date, date};
use jiff::{SignedDuration, Timestamp};
use rusqlite::Connection;
use rusqlite::types::Value as SqlValue;
use serde_json::{Value, json};
use worktime_domain::accounting::{
    AccountingLabel, AdjustmentId, LongLockReviewId, OperationKey, PowerState, RecordedTransition,
    SessionState, TrackingState, WorkdayTransition, WorkdayTransitionKind,
};
use worktime_domain::adjustments::{AdjustmentKind, ManualAdjustment, ManualClassification};
use worktime_domain::credit::{
    CreditLedgerEntry, CreditLedgerEntryId, CreditLedgerEntryKind, CreditOrigin,
};
use worktime_domain::history::{
    DailySummaryBreakdown, DailySummaryBreakdownEntry, DailySummaryDisposition,
    DailySummaryRevision, DailySummarySnapshot, DailySummarySnapshotParts,
};
use worktime_domain::intervals::UtcInterval;
use worktime_domain::notifications::NotificationTrigger;
use worktime_domain::sources::{
    ActivitySourceKind, ActivitySourceSelection, IdleSpan, IdleState, SourceQuality,
};
use worktime_domain::summaries::IndicatorState;
use worktime_domain::time::{FrozenClock, LocalWorkday, WorkdayWindow};

use crate::activity::{ActivitySourceSelectionRepository, ActivitySpanRepository};
use crate::adjustments::{ManualAdjustmentError, ManualAdjustmentRepository};
use crate::codec::{format_date, format_instant, from_ticks, to_ticks};
use crate::credit::{
    CreditLedgerError, CreditLedgerRepository, WeeklyCreditError, WeeklyCreditRepository,
    WeeklyRevisionVector,
};
use crate::history::{DailySummaryError, DailySummaryRepository, serialize_breakdown};
use crate::notifications::{NotificationClaim, NotificationReceiptRepository};
use crate::reviews::{
    LongLockResolutionAction, LongLockResolutionActionKind, LongLockResolutionClaim,
    LongLockResolutionError, LongLockResolutionRepository,
};
use crate::sessions::{AtomicWorkdayTransition, SessionEventError, SessionEventRepository};
use crate::test_support::{TemporaryDatabase, fixture_path};

const ZONE: &str = "Romance Standard Time";

fn dotnet(section: &str) -> Value {
    let text = std::fs::read_to_string(fixture_path("repositories.json")).unwrap();
    serde_json::from_str::<Value>(&text).unwrap()[section].take()
}

fn utc(text: &str) -> Timestamp {
    text.parse().unwrap()
}

/// Every row of each table, in insertion order, as the .NET generator dumps them.
fn rows(connection: &Connection, tables: &[&str]) -> Value {
    let dump = tables.iter().map(|table| {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid;"))
            .unwrap();
        let columns = statement.column_count();
        let rows: Vec<Value> = statement
            .query_map([], |row| {
                (0..columns)
                    .map(|column| {
                        Ok(match row.get::<_, SqlValue>(column)? {
                            SqlValue::Null => Value::Null,
                            SqlValue::Integer(number) => json!(number),
                            SqlValue::Text(text) => json!(text),
                            other => panic!("{table} holds {other:?}"),
                        })
                    })
                    .collect::<rusqlite::Result<Value>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        ((*table).to_owned(), Value::Array(rows))
    });
    Value::Object(dump.collect())
}

#[test]
fn session_events_match_the_dotnet_repository() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(utc("2026-08-30T12:00:00.1234567Z"));
    let repository = SessionEventRepository::new(&database.connection, &clock);
    let step = |key: &str, at: &str, kind| AtomicWorkdayTransition {
        operation_key: OperationKey::new(key).unwrap(),
        transition: WorkdayTransition::new(utc(at), ZONE, kind).unwrap(),
    };
    let session = |key, at, state| step(key, at, WorkdayTransitionKind::SessionObserved(state));
    let power = |key, at, state| step(key, at, WorkdayTransitionKind::PowerObserved(state));
    let tracking = |key, at, state| step(key, at, WorkdayTransitionKind::TrackingChanged(state));
    let batches = [
        vec![session(
            "s-unknown",
            "2026-08-29T21:00:00Z",
            SessionState::Unknown,
        )],
        vec![session(
            "s-lock",
            "2026-08-29T22:00:00.0000001Z",
            SessionState::Locked,
        )],
        vec![session(
            "s-lock-again",
            "2026-08-29T23:30:00Z",
            SessionState::Locked,
        )],
        vec![
            power("p-unknown", "2026-08-30T06:00:00Z", PowerState::Unknown),
            power("p-suspend", "2026-08-30T06:00:00Z", PowerState::Suspended),
            power("p-resume", "2026-08-30T07:15:30.5Z", PowerState::Active),
        ],
        vec![session(
            "s-unlock",
            "2026-08-30T07:20:00.9999999Z",
            SessionState::Unlocked,
        )],
        vec![
            tracking("t-pause", "2026-08-30T12:00:00Z", TrackingState::Paused),
            tracking("t-resume", "2026-08-30T12:45:00Z", TrackingState::Running),
            tracking("t-finish", "2026-08-30T17:30:00Z", TrackingState::Finished),
        ],
        vec![session(
            "s-lock",
            "2026-08-29T22:00:00.0000001Z",
            SessionState::Locked,
        )],
        vec![session(
            "s-lock",
            "2026-08-29T22:00:00.0000002Z",
            SessionState::Locked,
        )],
        vec![
            session("s-late-lock", "2026-08-30T18:00:00Z", SessionState::Locked),
            session("s-unlock", "2026-08-30T07:20:00Z", SessionState::Unlocked),
        ],
    ];

    let mut results = Vec::new();
    for batch in &batches {
        match repository.record_atomically(batch) {
            Ok(recorded) => results.extend(
                recorded
                    .iter()
                    .map(|result| format!("{:?} {}", result.outcome, result.authoritative_id.0)),
            ),
            Err(SessionEventError::OperationConflict(key)) => {
                results.push(format!("conflict {key}"));
            }
            Err(error) => panic!("{error}"),
        }
    }
    let history = repository
        .read_for_accounting(&WorkdayWindow {
            workday: LocalWorkday::new(jiff::civil::date(2026, 8, 30), ZONE),
            start: utc("2026-08-30T00:00:00Z"),
            end: utc("2026-08-31T00:00:00Z"),
        })
        .unwrap();
    let render = |transition: RecordedTransition| {
        let (lane, state) = match &transition {
            RecordedTransition::Session(recorded) => ("session", format!("{:?}", recorded.state)),
            RecordedTransition::Power(recorded) => ("power", format!("{:?}", recorded.state)),
            RecordedTransition::Tracking(recorded) => ("tracking", format!("{:?}", recorded.state)),
        };
        format!(
            "{lane} {} {} {state} {}",
            transition.id().0,
            format_instant(transition.occurred_at()),
            transition.origin().source_id()
        )
    };
    let anchors = history.before_window().clone();
    let before = [
        anchors.session.map(RecordedTransition::from),
        anchors.power.map(RecordedTransition::from),
        anchors.tracking.map(RecordedTransition::from),
    ]
    .into_iter()
    .flatten()
    .map(|anchor| format!("before {}", render(anchor)));
    let in_window = history
        .in_window()
        .iter()
        .map(|transition| format!("in {}", render(transition.clone())));

    assert_eq!(
        json!({
            "results": results,
            "history": before.chain(in_window).collect::<Vec<_>>(),
            "rows": rows(&database.connection, &["SessionEvent"]),
        }),
        dotnet("sessionEvents")
    );
}

#[test]
fn manual_adjustments_match_the_dotnet_repository() {
    let database = TemporaryDatabase::new();
    let repository = ManualAdjustmentRepository::new(&database.connection);
    let friday = LocalWorkday::new(jiff::civil::date(2026, 8, 28), ZONE);
    let id = |value: &str| AdjustmentId::new(value).unwrap();
    let between = |start, end| UtcInterval::new(utc(start), utc(end)).unwrap();
    let adjustment = |value, workday: &LocalWorkday, created_at, supersedes: Option<&str>, kind| {
        ManualAdjustment::new(
            id(value),
            workday.clone(),
            utc(created_at),
            supersedes.map(id),
            kind,
        )
        .unwrap()
    };
    let minutes = |minutes| AdjustmentKind::DurationWork {
        duration: SignedDuration::from_mins(minutes),
    };
    let duration_created_at = "2026-08-28T12:00:00.1234567Z";
    let steps = [
        adjustment("duration", &friday, duration_created_at, None, minutes(35))
            .with_reason("  Missing meeting  "),
        adjustment(
            "interval",
            &friday,
            "2026-08-28T12:00:00.5Z",
            None,
            AdjustmentKind::IntervalWork {
                interval: between("2026-08-28T07:00:00Z", "2026-08-28T08:00:00Z"),
            },
        )
        .with_reason("Early start"),
        adjustment(
            "exclude",
            &friday,
            "2026-08-28T12:01:00Z",
            None,
            AdjustmentKind::Exclusion {
                interval: between("2026-08-28T08:00:00Z", "2026-08-28T09:00:00.0000001Z"),
            },
        ),
        adjustment(
            "confirm",
            &friday,
            "2026-08-28T12:02:00Z",
            Some("exclude"),
            AdjustmentKind::Reclassification {
                interval: between("2026-08-28T09:00:00Z", "2026-08-28T10:00:00Z"),
                classification: ManualClassification::Credited,
                confirms_review_id: Some(LongLockReviewId::new("session-lock:42").unwrap()),
            },
        )
        .with_reason("Confirmed"),
        adjustment(
            "reclassify-excluded",
            &friday,
            "2026-08-28T12:03:00Z",
            None,
            AdjustmentKind::Reclassification {
                interval: between("2026-08-28T10:00:00Z", "2026-08-28T11:00:00Z"),
                classification: ManualClassification::Excluded,
                confirms_review_id: None,
            },
        ),
        adjustment(
            "revoke",
            &friday,
            "2026-08-28T12:04:00Z",
            Some("interval"),
            AdjustmentKind::Revocation,
        )
        .with_reason("Mistake"),
        adjustment("duration", &friday, duration_created_at, None, minutes(35))
            .with_reason("Missing meeting"),
        adjustment("duration", &friday, duration_created_at, None, minutes(35))
            .with_reason("Another reason"),
        adjustment(
            "orphan",
            &friday,
            "2026-08-28T12:05:00Z",
            Some("missing"),
            minutes(10),
        ),
        adjustment(
            "revoke-again",
            &friday,
            "2026-08-28T12:06:00Z",
            Some("interval"),
            AdjustmentKind::Revocation,
        ),
        adjustment(
            "other-zone",
            &LocalWorkday::new(friday.date, "UTC"),
            "2026-08-28T12:07:00Z",
            None,
            minutes(15),
        ),
    ];

    let results: Vec<_> = steps
        .iter()
        .map(|step| match repository.append(step) {
            Ok(result) => format!("{:?} {}", result.outcome, result.authoritative_id),
            Err(ManualAdjustmentError::IdConflict(id)) => format!("id-conflict {id}"),
            Err(ManualAdjustmentError::ParentConflict {
                superseded_id,
                reason,
            }) => format!("parent-conflict {superseded_id} {reason:?}"),
            Err(ManualAdjustmentError::SupersessionConflict {
                superseded_id,
                existing_successor_id,
            }) => format!("supersession-conflict {superseded_id} {existing_successor_id}"),
            Err(error) => panic!("{error}"),
        })
        .collect();
    let read: Vec<_> = repository
        .read_for_workday(&friday)
        .unwrap()
        .iter()
        .map(|adjustment| adjustment.id().to_string())
        .collect();

    assert_eq!(
        json!({
            "results": results,
            "read": read,
            "rows": rows(&database.connection, &["ManualAdjustment"]),
        }),
        dotnet("manualAdjustments")
    );
}

#[test]
fn activity_spans_and_selections_match_the_dotnet_repositories() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(utc("2026-10-12T09:00:00.25Z"));
    let spans = ActivitySpanRepository::new(&database.connection, &clock);
    let span = |state, start, settled| IdleSpan::new(state, utc(start), utc(settled)).unwrap();
    let mut results = Vec::new();
    let active = spans
        .append(
            span(
                IdleState::Active,
                "2026-10-12T09:00:00Z",
                "2026-10-12T09:00:00Z",
            ),
            None,
        )
        .unwrap();
    results.push(format!("append {}", active.0));
    spans
        .advance_settled(active, utc("2026-10-12T09:00:10Z"))
        .unwrap();
    let idle = spans
        .append(
            span(
                IdleState::Idle,
                "2026-10-12T09:00:12Z",
                "2026-10-12T09:00:15Z",
            ),
            Some(active),
        )
        .unwrap();
    results.push(format!("append {}", idle.0));
    let replayed = spans
        .append(
            span(
                IdleState::Idle,
                "2026-10-12T09:00:12Z",
                "2026-10-12T09:00:12Z",
            ),
            Some(active),
        )
        .unwrap();
    results.push(format!("append {}", replayed.0));
    spans
        .advance_settled(idle, utc("2026-10-12T09:00:05Z"))
        .unwrap();
    let restarted = spans
        .append(
            span(
                IdleState::Active,
                "2026-10-12T09:30:00.0000001Z",
                "2026-10-12T09:31:00Z",
            ),
            None,
        )
        .unwrap();
    results.push(format!("append {}", restarted.0));

    let selections = ActivitySourceSelectionRepository::new(&database.connection, &clock);
    for _ in 0..2 {
        results.push(format!(
            "legacy {}",
            selections.append_legacy_selection().unwrap()
        ));
    }
    selections
        .append(ActivitySourceSelection {
            effective_from: utc("2026-10-12T08:00:00Z"),
            source: ActivitySourceKind::BuiltIn,
        })
        .unwrap();
    selections
        .append(ActivitySourceSelection {
            effective_from: utc("2026-10-12T10:00:00.0000001Z"),
            source: ActivitySourceKind::ActivityWatch,
        })
        .unwrap();

    let settled: Vec<_> = spans
        .read_settled(utc("2026-10-12T09:00:00Z"), utc("2026-10-12T10:00:00Z"))
        .unwrap()
        .into_iter()
        .map(|stored| {
            format!(
                "{} {:?} {} {}",
                stored.id.0,
                stored.span.state(),
                format_instant(stored.span.start()),
                format_instant(stored.span.settled_through())
            )
        })
        .collect();
    let render = |timeline: Vec<ActivitySourceSelection>| -> Vec<String> {
        timeline
            .into_iter()
            .map(|selection| {
                format!(
                    "{} {:?}",
                    format_instant(selection.effective_from),
                    selection.source
                )
            })
            .collect()
    };
    let timeline = selections
        .read_timeline(ActivitySourceSelection::LEGACY_EPOCH, None)
        .unwrap();
    let window = selections
        .read_timeline(
            utc("2026-10-12T09:00:00Z"),
            Some(utc("2026-10-12T10:00:00Z")),
        )
        .unwrap();

    assert_eq!(
        json!({
            "results": results,
            "settled": settled,
            "timeline": render(timeline),
            "window": render(window),
            "rows": rows(&database.connection, &["ActivitySpan", "ActivitySourceSelection"]),
        }),
        dotnet("activity")
    );
}

#[test]
fn long_lock_resolutions_match_the_dotnet_repository() {
    let database = TemporaryDatabase::new();
    let repository = LongLockResolutionRepository::new(&database.connection);
    let claim = |day, kind, identity, at| LongLockResolutionClaim {
        workday: LocalWorkday::new(jiff::civil::date(2026, 8, day), "UTC"),
        review_id: LongLockReviewId::new("session-lock:42").unwrap(),
        action: LongLockResolutionAction::new(kind, identity).unwrap(),
        proposed_created_at: utc(at),
    };
    use LongLockResolutionActionKind::{ConfirmWork, Exclude, FinishAtLockStart};
    let steps = [
        (
            false,
            claim(
                24,
                ConfirmWork,
                " confirm-42 ",
                "2026-08-24T12:00:00.1234567Z",
            ),
        ),
        (
            false,
            claim(24, ConfirmWork, "confirm-42", "2026-08-24T13:00:00Z"),
        ),
        (
            false,
            claim(24, Exclude, "exclude-42", "2026-08-24T13:00:00Z"),
        ),
        (
            false,
            claim(25, FinishAtLockStart, "finish-43", "2026-08-25T12:00:00Z"),
        ),
        (
            true,
            claim(24, ConfirmWork, "another-command", "2026-08-24T12:00:00Z"),
        ),
        (
            true,
            claim(25, FinishAtLockStart, "finish-43", "2026-08-25T12:00:00Z"),
        ),
        (
            false,
            claim(26, Exclude, "exclude-44", "2026-08-26T12:00:00Z"),
        ),
    ];

    let results: Vec<_> = steps
        .iter()
        .map(|(release, claim)| {
            if *release {
                return format!("released {}", repository.release(claim).unwrap());
            }
            match repository.claim(claim) {
                Ok(result) => format!(
                    "{:?} {}",
                    result.outcome,
                    format_instant(result.canonical_created_at)
                ),
                Err(LongLockResolutionError::Conflict(conflict)) => format!(
                    "conflict {:?} {}",
                    conflict.existing_action.kind(),
                    conflict.existing_action.identity()
                ),
                Err(error) => panic!("{error}"),
            }
        })
        .collect();

    assert_eq!(
        json!({
            "results": results,
            "rows": rows(&database.connection, &["LongLockResolution"]),
        }),
        dotnet("longLockResolutions")
    );
}

#[test]
fn notification_receipts_match_the_dotnet_repository() {
    let database = TemporaryDatabase::new();
    let repository = NotificationReceiptRepository::new(&database.connection);
    let claim = |trigger, eligible_at| NotificationClaim {
        workday: LocalWorkday::new(jiff::civil::date(2026, 9, 7), "UTC"),
        trigger,
        eligible_at: utc(eligible_at),
    };
    let steps = [
        (
            vec![
                claim(
                    NotificationTrigger::CompletionApproach,
                    "2026-09-07T17:45:00Z",
                ),
                claim(
                    NotificationTrigger::LatestDepartureApproach,
                    "2026-09-07T18:00:00Z",
                ),
            ],
            "delivery-1",
            "2026-09-07T17:45:00.5Z",
        ),
        (
            vec![
                claim(
                    NotificationTrigger::CompletionApproach,
                    "2026-09-07T17:45:00Z",
                ),
                claim(NotificationTrigger::DayComplete, "2026-09-07T19:00:00Z"),
                claim(NotificationTrigger::DayComplete, "2026-09-07T19:30:00Z"),
            ],
            "delivery-2",
            "2026-09-07T19:00:00Z",
        ),
    ];

    let results: Vec<_> = steps
        .iter()
        .map(|(claims, delivery_id, reserved_at)| {
            repository
                .try_reserve(claims, delivery_id, utc(reserved_at))
                .unwrap()
                .iter()
                .map(|trigger| format!("{trigger:?}"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();

    assert_eq!(
        json!({
            "results": results,
            "rows": rows(&database.connection, &["NotificationReceipt"]),
        }),
        dotnet("notificationReceipts")
    );
}

const ODD_ADJUSTMENT_ID: &str = "adjustment \"quoted\" <tag> & 'apostrophe' +plus `tick` back\\slash /slash \u{e9} \u{1F600} \u{2028} \u{7f}";

const ODD_REVIEW_ID: &str = "review\u{1}\t\n\r\u{8}\u{c}\u{1f} \u{a0}\u{feff}";

fn ticks(ticks: i64) -> SignedDuration {
    from_ticks(ticks)
}

fn at_hour(day: Date, hour: i8) -> Timestamp {
    day.at(hour, 0, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn plus_days(day: Date, days: i64) -> Date {
    day + jiff::Span::new().days(days)
}

/// `DetailedRevision` of the .NET scenario: every disposition and label, intervals at tick
/// precision and ids that need escaping.
fn detailed_revision(
    day: Date,
    revision: i64,
    fingerprint: &str,
    complete: bool,
    indicator_state: IndicatorState,
) -> DailySummaryRevision {
    let start = at_hour(day, 8) + ticks(1);
    let interval = |hours: i64, length: i64| {
        let from = start + SignedDuration::from_hours(hours);
        Some(UtcInterval::new(from, from + ticks(length)).unwrap())
    };
    let entry = |disposition,
                 label,
                 duration,
                 interval,
                 adjustment_id: Option<&str>,
                 review_id: Option<&str>| {
        DailySummaryBreakdownEntry::new(
            disposition,
            label,
            ticks(duration),
            interval,
            adjustment_id.map(|id| AdjustmentId::new(id).unwrap()),
            review_id.map(|id| LongLockReviewId::new(id).unwrap()),
        )
        .unwrap()
    };
    use AccountingLabel as Label;
    use DailySummaryDisposition as Disposition;
    let entries = [
        entry(
            Disposition::ManualCredit,
            Label::Manual,
            13,
            None,
            Some(ODD_ADJUSTMENT_ID),
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
            Some(ODD_REVIEW_ID),
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
        entry(
            Disposition::Excluded,
            Label::LunchLock,
            1,
            interval(5, 1),
            None,
            None,
        ),
        entry(
            Disposition::Excluded,
            Label::Suspended,
            1,
            interval(6, 1),
            None,
            None,
        ),
        entry(
            Disposition::Excluded,
            Label::ManualPause,
            1,
            interval(7, 1),
            None,
            None,
        ),
        entry(
            Disposition::Excluded,
            Label::DayFinished,
            1,
            interval(8, 1),
            None,
            None,
        ),
    ];
    let snapshot = DailySummarySnapshot::new(DailySummarySnapshotParts {
        workday: LocalWorkday::new(day, ZONE),
        computed_at: at_hour(day, 12) + ticks(17),
        input_fingerprint: fingerprint.to_owned(),
        credited_work: ticks(21),
        excluded_work: ticks(11),
        unknown_work: ticks(11),
        nominal_target: ticks(if complete { 22 } else { 1_001 }),
        effective_target: ticks(if complete { 21 } else { 1_000 }),
        carry_available: ticks(29),
        carry_applied: ticks(1),
        carry_earned: ticks(31),
        carry_expired: ticks(37),
        carry_debt: ticks(41),
        debt_pending: !complete,
        projected_departure: (!complete).then(|| at_hour(day, 18) + ticks(19)),
        indicator_state,
        source_quality: if complete {
            SourceQuality::Complete
        } else {
            SourceQuality::Ambiguous
        },
        has_pending_review: !complete,
        breakdown: DailySummaryBreakdown::new(entries),
    })
    .unwrap();
    DailySummaryRevision::new(revision, snapshot).unwrap()
}

/// `WriteRevision` of the .NET scenario: every field of a snapshot, its breakdown as the codec
/// writes it.
fn render_revision(revision: Option<DailySummaryRevision>) -> Value {
    let Some(revision) = revision else {
        return Value::Null;
    };
    let snapshot = revision.snapshot();
    let durations = [
        snapshot.credited_work(),
        snapshot.excluded_work(),
        snapshot.unknown_work(),
        snapshot.nominal_target(),
        snapshot.effective_target(),
        snapshot.carry_available(),
        snapshot.carry_applied(),
        snapshot.carry_earned(),
        snapshot.carry_expired(),
        snapshot.carry_debt(),
    ];
    json!({
        "workday": format!("{} {}", format_date(snapshot.workday().date), snapshot.workday().time_zone_id),
        "revision": revision.revision(),
        "computedAt": format_instant(snapshot.computed_at()),
        "inputFingerprint": snapshot.input_fingerprint(),
        "ticks": durations.map(|duration| to_ticks(duration).unwrap()),
        "projectedDeparture": snapshot.projected_departure().map(format_instant),
        "indicatorState": format!("{:?}", snapshot.indicator_state()),
        "sourceQuality": format!("{:?}", snapshot.source_quality()),
        "hasPendingReview": snapshot.has_pending_review(),
        "debtPending": snapshot.debt_pending(),
        "breakdown": serialize_breakdown(snapshot.breakdown()).unwrap(),
    })
}

/// `DailySummaries` of the .NET scenario; the credit scenario runs after it on the same database.
fn replay_daily_summaries(connection: &Connection) -> Value {
    let repository = DailySummaryRepository::new(connection);
    let monday = date(2026, 9, 7);
    let steps = [
        detailed_revision(monday, 1, "monday-first", false, IndicatorState::Green),
        detailed_revision(monday, 1, "monday-first", false, IndicatorState::Green),
        detailed_revision(monday, 1, "monday-different", false, IndicatorState::Green),
        detailed_revision(monday, 3, "monday-gap", false, IndicatorState::Orange),
        detailed_revision(monday, 2, "monday-second", false, IndicatorState::Red),
        detailed_revision(
            plus_days(monday, 1),
            2,
            "tuesday-missing-first",
            false,
            IndicatorState::Green,
        ),
        detailed_revision(
            plus_days(monday, 2),
            1,
            "wednesday",
            true,
            IndicatorState::Blue,
        ),
        detailed_revision(
            plus_days(monday, 4),
            1,
            "friday",
            false,
            IndicatorState::Orange,
        ),
        detailed_revision(
            plus_days(monday, 7),
            1,
            "next-monday",
            true,
            IndicatorState::Blue,
        ),
    ];

    let results: Vec<_> = steps
        .iter()
        .map(|revision| match repository.append(revision) {
            Ok(result) => format!("{:?} {}", result.outcome, result.authoritative_revision),
            Err(DailySummaryError::RevisionConflict {
                workday_date,
                revision,
            }) => format!("conflict {} {revision}", format_date(workday_date)),
            Err(DailySummaryError::RevisionSequence {
                workday_date,
                expected_revision,
                attempted_revision,
            }) => format!(
                "sequence {} {expected_revision} {attempted_revision}",
                format_date(workday_date)
            ),
            Err(error) => panic!("{error}"),
        })
        .collect();
    let latest: Vec<_> = [monday, plus_days(monday, 1), plus_days(monday, 2)]
        .into_iter()
        .map(|day| render_revision(repository.read_latest(day).unwrap()))
        .collect();
    let week: Vec<_> = repository
        .read_latest_week(plus_days(monday, 6))
        .unwrap()
        .into_iter()
        .map(|revision| render_revision(Some(revision)))
        .collect();
    json!({ "results": results, "latest": latest, "week": week })
}

#[test]
fn daily_summaries_match_the_dotnet_repository() {
    let database = TemporaryDatabase::new();

    let mut scenario = replay_daily_summaries(&database.connection);

    scenario["rows"] = rows(&database.connection, &["DailySummaryRevision"]);
    assert_eq!(scenario, dotnet("dailySummaries"));
}

/// `WeeklySummary` of the .NET scenario: whole minutes against a 7-hour target.
#[derive(Clone, Copy)]
struct WeeklySummary {
    day: Date,
    revision: i64,
    worked: i64,
    applied: i64,
    earned: i64,
    fingerprint: &'static str,
    available: Option<i64>,
    expired: i64,
}

impl WeeklySummary {
    fn new(day: Date, revision: i64, worked: i64) -> Self {
        Self {
            day,
            revision,
            worked,
            applied: 0,
            earned: 0,
            fingerprint: "facts",
            available: None,
            expired: 0,
        }
    }

    fn build(self) -> DailySummaryRevision {
        let minutes = SignedDuration::from_mins;
        let complete = self.worked >= 420 - self.applied;
        let computed_at = at_hour(self.day, 16);
        let breakdown = if self.worked == 0 {
            DailySummaryBreakdown::default()
        } else {
            DailySummaryBreakdown::new([DailySummaryBreakdownEntry::new(
                DailySummaryDisposition::ConfirmedCredit,
                AccountingLabel::ActiveWork,
                minutes(self.worked),
                None,
                None,
                None,
            )
            .unwrap()])
        };
        let snapshot = DailySummarySnapshot::new(DailySummarySnapshotParts {
            workday: LocalWorkday::new(self.day, "Europe/Paris"),
            computed_at,
            input_fingerprint: self.fingerprint.to_owned(),
            credited_work: minutes(self.worked),
            excluded_work: SignedDuration::ZERO,
            unknown_work: SignedDuration::ZERO,
            nominal_target: minutes(420),
            effective_target: minutes(420 - self.applied),
            carry_available: minutes(self.available.unwrap_or(self.earned)),
            carry_applied: minutes(self.applied),
            carry_earned: minutes(self.earned),
            carry_expired: minutes(self.expired),
            carry_debt: SignedDuration::ZERO,
            debt_pending: false,
            projected_departure: (!complete).then(|| computed_at + SignedDuration::from_hours(1)),
            indicator_state: if complete {
                IndicatorState::Blue
            } else {
                IndicatorState::Green
            },
            source_quality: SourceQuality::Complete,
            has_pending_review: false,
            breakdown,
        })
        .unwrap();
        DailySummaryRevision::new(self.revision, snapshot).unwrap()
    }
}

fn render_entry(entry: &CreditLedgerEntry) -> String {
    format!(
        "{}|{}|{:?}|{}|{}|{}|{}",
        entry.id().as_str(),
        format_date(entry.workday_date()),
        entry.kind(),
        to_ticks(entry.amount()).unwrap(),
        format_date(entry.origin().workday_date),
        entry.origin().revision,
        format_instant(entry.created_at())
    )
}

fn render_vector(vector: &WeeklyRevisionVector) -> String {
    vector
        .revisions()
        .map(|revision| revision.to_string())
        .join(",")
}

#[test]
fn credit_ledger_and_weekly_commits_match_the_dotnet_repositories() {
    let database = TemporaryDatabase::new();
    replay_daily_summaries(&database.connection);
    let ledger = CreditLedgerRepository::new(&database.connection);
    let weekly = WeeklyCreditRepository::new(&database.connection);
    let ledger_week = date(2026, 10, 5);
    let monday = date(2026, 9, 21);
    let day = |offset| plus_days(monday, offset);
    use CreditLedgerEntryKind::{Applied, Earned, Expired, Reversed};
    let ledger_entry = |id: &str, workday: Date, kind, amount: i64, origin: Date, revision| {
        CreditLedgerEntry::new(
            CreditLedgerEntryId::new(id).unwrap(),
            workday,
            kind,
            ticks(amount),
            CreditOrigin {
                workday_date: origin,
                revision,
            },
            at_hour(workday, 16) + ticks(amount),
        )
        .unwrap()
    };
    let movement = |id: &str, workday: i64, kind, minutes: i64, origin: i64, revision| {
        CreditLedgerEntry::new(
            CreditLedgerEntryId::new(id).unwrap(),
            day(workday),
            kind,
            SignedDuration::from_mins(minutes),
            CreditOrigin {
                workday_date: day(origin),
                revision,
            },
            at_hour(day(workday), 16),
        )
        .unwrap()
    };
    let mut results = Vec::new();
    let ledger_result = |result: Result<(), CreditLedgerError>| match result {
        Ok(()) => "done".to_owned(),
        Err(CreditLedgerError::NotAMonday(_)) => "argument weekStartDate".to_owned(),
        Err(CreditLedgerError::EntryOutsideWeek(_)) => "argument entries".to_owned(),
        Err(error) => panic!("{error}"),
    };
    let plus_ledger_days = |days| plus_days(ledger_week, days);
    results.push(ledger_result(ledger.replace_week(
        ledger_week,
        &[
            ledger_entry(
                " expired ",
                plus_ledger_days(4),
                Expired,
                4,
                ledger_week,
                14,
            ),
            ledger_entry("applied", plus_ledger_days(1), Applied, 1, ledger_week, 12),
            ledger_entry(
                "reversed",
                plus_ledger_days(2),
                Reversed,
                3,
                plus_ledger_days(1),
                13,
            ),
            ledger_entry("z-earned", ledger_week, Earned, 2, ledger_week, 11),
            ledger_entry("a-earned", ledger_week, Earned, 2, ledger_week, 10),
        ],
    )));
    results.push(ledger_result(ledger.replace_week(plus_ledger_days(1), &[])));
    results.push(ledger_result(ledger.replace_week(
        ledger_week,
        &[ledger_entry(
            "other-week",
            plus_ledger_days(7),
            Earned,
            1,
            plus_ledger_days(7),
            1,
        )],
    )));

    let canonical = [
        movement("expire", 4, Expired, 20, 1, 1),
        movement("apply-tuesday", 2, Applied, 10, 1, 1),
        movement("earn-tuesday", 1, Earned, 30, 1, 1),
        movement("apply-monday", 2, Applied, 30, 0, 1),
        movement("earn-monday", 0, Earned, 30, 0, 1),
    ];
    let summary = WeeklySummary::new;
    let week = [
        WeeklySummary {
            earned: 30,
            ..summary(day(0), 1, 450)
        }
        .build(),
        WeeklySummary {
            earned: 30,
            available: Some(60),
            ..summary(day(1), 1, 450)
        }
        .build(),
        WeeklySummary {
            applied: 40,
            available: Some(20),
            ..summary(day(2), 1, 380)
        }
        .build(),
        WeeklySummary {
            available: Some(0),
            expired: 20,
            ..summary(day(4), 1, 420)
        }
        .build(),
    ];
    let with = |base: &[CreditLedgerEntry], extra: &[CreditLedgerEntry]| [base, extra].concat();
    let commit = |expected: &WeeklyRevisionVector,
                  monday: Date,
                  revisions: &[DailySummaryRevision],
                  replacement: &[CreditLedgerEntry]| {
        match weekly.commit(monday, expected, revisions, replacement) {
            Ok(()) => "done".to_owned(),
            Err(WeeklyCreditError::Stale { expected, actual }) => {
                format!(
                    "stale {} {}",
                    render_vector(&expected),
                    render_vector(&actual)
                )
            }
            Err(WeeklyCreditError::Inconsistent(inconsistency)) => {
                format!("inconsistent {inconsistency}")
            }
            Err(WeeklyCreditError::Summary(DailySummaryError::RevisionConflict {
                workday_date,
                revision,
            })) => format!("conflict {} {revision}", format_date(workday_date)),
            Err(WeeklyCreditError::NotAMonday(_)) => "argument weekStartDate".to_owned(),
            Err(WeeklyCreditError::VectorOfAnotherWeek(_)) => {
                "argument expectedRevisions".to_owned()
            }
            Err(
                WeeklyCreditError::RevisionOutsideWeek { .. }
                | WeeklyCreditError::DuplicateRevision { .. },
            ) => "argument revisions".to_owned(),
            Err(WeeklyCreditError::EntryOutsideWeek(_)) => "argument entries".to_owned(),
            Err(error) => panic!("{error}"),
        }
    };
    let empty = weekly.read_revision_vector(monday).unwrap();
    results.push(commit(&empty, monday, &week, &canonical));
    results.push(commit(&empty, monday, &week, &canonical));
    let committed = weekly.read_revision_vector(monday).unwrap();
    let steps: [(Vec<DailySummaryRevision>, Vec<CreditLedgerEntry>); 15] = [
        (week.to_vec(), canonical.to_vec()),
        (
            vec![
                WeeklySummary {
                    earned: 30,
                    fingerprint: "different",
                    ..summary(day(0), 1, 450)
                }
                .build(),
            ],
            canonical.to_vec(),
        ),
        (
            vec![
                WeeklySummary {
                    available: Some(20),
                    ..summary(day(3), 1, 420)
                }
                .build(),
            ],
            canonical[1..].to_vec(),
        ),
        (
            vec![
                WeeklySummary {
                    earned: 20,
                    ..summary(day(0), 2, 440)
                }
                .build(),
            ],
            canonical.to_vec(),
        ),
        (
            vec![
                WeeklySummary {
                    available: Some(10),
                    expired: 10,
                    ..summary(day(4), 2, 420)
                }
                .build(),
            ],
            with(&canonical[1..], &[movement("expire", 4, Expired, 10, 1, 1)]),
        ),
        (
            vec![
                WeeklySummary {
                    applied: 40,
                    available: Some(25),
                    ..summary(day(2), 2, 380)
                }
                .build(),
            ],
            canonical.to_vec(),
        ),
        (
            vec![],
            with(&canonical, &[movement("reversed", 3, Reversed, 1, 0, 1)]),
        ),
        (
            vec![],
            with(&canonical, &[movement("early", 0, Applied, 1, 0, 1)]),
        ),
        (
            vec![],
            with(
                &canonical[1..],
                &[movement("thursday-expiry", 3, Expired, 20, 1, 1)],
            ),
        ),
        (
            vec![],
            with(
                &canonical[..4],
                &[movement("earn-monday", 0, Earned, 30, 1, 1)],
            ),
        ),
        (
            vec![],
            with(&canonical, &[movement("no-summary", 3, Earned, 5, 3, 1)]),
        ),
        (
            vec![],
            with(
                &canonical[..4],
                &[movement("earn-monday", 0, Earned, 30, 0, 2)],
            ),
        ),
        (
            vec![],
            vec![
                canonical[0].clone(),
                canonical[2].clone(),
                canonical[4].clone(),
                movement("apply-tuesday", 2, Applied, 10, 0, 1),
                movement("apply-monday", 2, Applied, 30, 1, 1),
            ],
        ),
        (
            vec![],
            vec![
                canonical[0].clone(),
                canonical[2].clone(),
                canonical[4].clone(),
                movement("apply-all", 2, Applied, 70, 0, 1),
            ],
        ),
        (vec![summary(day(7), 1, 420).build()], vec![]),
    ];
    for (revisions, replacement) in &steps {
        results.push(commit(&committed, monday, revisions, replacement));
    }
    results.push(commit(
        &committed,
        monday,
        &[
            summary(day(1), 2, 450).build(),
            summary(day(1), 2, 450).build(),
        ],
        &[],
    ));
    results.push(commit(
        &committed,
        monday,
        &[],
        &[movement("next-week", 7, Earned, 1, 7, 1)],
    ));
    results.push(commit(&committed, day(1), &[], &[]));
    results.push(commit(
        &WeeklyRevisionVector::new(day(7), [0; 5]).unwrap(),
        monday,
        &[],
        &[],
    ));

    let ledger_read: Vec<_> = ledger
        .read_week(ledger_week)
        .unwrap()
        .iter()
        .map(render_entry)
        .collect();
    let state = weekly.read_week(monday).unwrap();
    let latest_revisions: Vec<_> = state
        .latest_revisions()
        .iter()
        .map(|revision| {
            format!(
                "{} {} {}",
                format_date(revision.workday_date()),
                revision.revision(),
                to_ticks(revision.snapshot().carry_available()).unwrap()
            )
        })
        .collect();
    let pending_expiry: Vec<_> = [date(2026, 9, 7), day(0), day(7), date(2026, 10, 12)]
        .into_iter()
        .map(|before| {
            weekly
                .read_pending_expiry_weeks(before)
                .unwrap()
                .into_iter()
                .map(format_date)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();

    assert_eq!(
        json!({
            "results": results,
            "ledger": ledger_read,
            "vector": render_vector(&state.revision_vector()),
            "latestRevisions": latest_revisions,
            "weekLedger": state.ledger().iter().map(render_entry).collect::<Vec<_>>(),
            "pendingExpiry": pending_expiry,
            "rows": rows(&database.connection, &["DailySummaryRevision", "CreditLedgerEntry"]),
        }),
        dotnet("credit")
    );
}
