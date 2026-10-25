use jiff::civil::date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use super::{Type, manual_id, minutes, origin, shape, shapes};
use crate::accounting::{
    AccountingInput, AccountingLabel, AccountingPolicy, AccountingResult, ActivityCoverage,
    ActivitySegment, ActivityState, AdjustmentId, LongLockReviewId, LongLockReviewReason,
    PowerState, Recorded, RecordedTransition, SegmentKind, SessionState, WorkdayEventHistory,
    WorkdayEventId, WorkdayStateAnchors, calculate,
};
use crate::adjustments::{ManualAdjustment, validate_for_append};
use crate::intervals::UtcInterval;
use crate::time::{LocalWorkday, WorkdayWindow};

fn at(day: i8, hour: i8, minute: i8) -> Timestamp {
    date(2026, 10, day)
        .at(hour, minute, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn interval(start: Timestamp, end: Timestamp) -> UtcInterval {
    UtcInterval::new(start, end).unwrap()
}

fn monday() -> LocalWorkday {
    LocalWorkday::new(date(2026, 10, 5), "UTC")
}

fn tuesday() -> LocalWorkday {
    LocalWorkday::new(date(2026, 10, 6), "UTC")
}

fn lock_review_id() -> LongLockReviewId {
    LongLockReviewId::new("session-lock:355").unwrap()
}

fn evening_lock() -> Recorded<SessionState> {
    Recorded::new(
        WorkdayEventId(355),
        at(5, 16, 46),
        SessionState::Locked,
        origin("windows:session"),
    )
}

fn unlock_at(occurred_at: Timestamp) -> RecordedTransition {
    Recorded::new(
        WorkdayEventId(356),
        occurred_at,
        SessionState::Unlocked,
        origin("windows:session"),
    )
    .into()
}

fn calculate_monday(
    now: Timestamp,
    in_window: Vec<RecordedTransition>,
    adjustments: Vec<ManualAdjustment>,
) -> AccountingResult {
    calculate_day(
        WorkdayWindow {
            workday: monday(),
            start: at(5, 0, 0),
            end: at(6, 0, 0),
        },
        now,
        WorkdayEventHistory::new(WorkdayStateAnchors::default(), in_window),
        interval(at(5, 8, 0), at(5, 16, 46)),
        adjustments,
    )
}

fn calculate_tuesday(
    now: Timestamp,
    in_window: Vec<RecordedTransition>,
    adjustments: Vec<ManualAdjustment>,
) -> AccountingResult {
    calculate_day(
        WorkdayWindow {
            workday: tuesday(),
            start: at(6, 0, 0),
            end: at(7, 0, 0),
        },
        now,
        WorkdayEventHistory::new(
            WorkdayStateAnchors {
                session: Some(evening_lock()),
                ..WorkdayStateAnchors::default()
            },
            in_window,
        ),
        interval(at(6, 0, 0), at(7, 0, 0)),
        adjustments,
    )
}

fn calculate_day(
    window: WorkdayWindow,
    now: Timestamp,
    history: WorkdayEventHistory,
    active: UtcInterval,
    adjustments: Vec<ManualAdjustment>,
) -> AccountingResult {
    let bounds = interval(window.start, window.end);
    let afk = |start, end| {
        ActivitySegment::new(
            interval(start, end),
            ActivityState::Afk,
            origin("activitywatch:afk"),
        )
    };
    let mut segments = Vec::new();
    if active.start() > bounds.start() {
        segments.push(afk(bounds.start(), active.start()));
    }
    segments.push(ActivitySegment::new(
        active,
        ActivityState::Active,
        origin("activitywatch:afk"),
    ));
    if active.end() < bounds.end() {
        segments.push(afk(active.end(), bounds.end()));
    }
    let input = AccountingInput::new(
        window,
        now,
        AccountingPolicy::new(interval(at(5, 12, 0), at(5, 13, 30)), minutes(90)).unwrap(),
        ActivityCoverage::new(bounds, segments),
        history,
        adjustments,
    )
    .unwrap();
    calculate(&input).unwrap()
}

#[test]
fn ended_day_with_lock_open_at_window_end_excludes_it_as_overnight_lock() {
    let result = calculate_monday(at(6, 10, 0), vec![evening_lock().into()], vec![]);

    let overnight = result.timeline().segments().last().unwrap();
    assert_eq!(
        shape(overnight),
        (
            Type::Excluded,
            interval(at(5, 16, 46), at(6, 0, 0)),
            AccountingLabel::OvernightLock
        )
    );
    assert_eq!(result.timeline().provisional_work(), SignedDuration::ZERO);
    assert_eq!(result.credited_work(), minutes(526));
    assert!(result.pending_reviews().is_empty());
}

#[test]
fn next_day_excludes_lock_anchored_before_the_window_until_unlock() {
    let result = calculate_tuesday(at(6, 7, 7), vec![unlock_at(at(6, 6, 57))], vec![]);

    assert_eq!(
        shapes(&result),
        [
            (
                Type::Excluded,
                interval(at(6, 0, 0), at(6, 6, 57)),
                AccountingLabel::OvernightLock
            ),
            (
                Type::Credited,
                interval(at(6, 6, 57), at(6, 7, 7)),
                AccountingLabel::ActiveWork
            ),
        ]
    );
    assert_eq!(result.timeline().provisional_work(), SignedDuration::ZERO);
    assert_eq!(result.credited_work(), minutes(10));
    assert!(result.pending_reviews().is_empty());
}

#[test]
fn next_day_keeps_lock_anchored_before_the_window_excluded_while_still_locked() {
    let result = calculate_tuesday(at(6, 9, 0), vec![], vec![]);

    assert_eq!(
        shapes(&result),
        [(
            Type::Excluded,
            interval(at(6, 0, 0), at(6, 9, 0)),
            AccountingLabel::OvernightLock
        )]
    );
    assert_eq!(result.credited_work(), SignedDuration::ZERO);
    assert!(result.pending_reviews().is_empty());
}

#[test]
fn live_day_keeps_long_open_lock_provisional_with_an_open_review() {
    let result = calculate_monday(at(5, 18, 30), vec![evening_lock().into()], vec![]);

    let locked = result.timeline().segments().last().unwrap();
    assert!(matches!(
        locked.kind(),
        SegmentKind::ProvisionalCredit { .. }
    ));
    assert_eq!(locked.label(), AccountingLabel::LockedWork);
    assert_eq!(result.timeline().provisional_work(), minutes(104));
    let [review] = result.pending_reviews() else {
        panic!("expected one review");
    };
    assert_eq!(review.id, lock_review_id());
    assert_eq!(review.reason, LongLockReviewReason::ThresholdExceeded);
    assert!(review.is_open);
}

#[test]
fn in_window_unlock_keeps_provisional_credit_and_a_closed_review_after_the_day_ends() {
    let result = calculate_monday(
        at(6, 10, 0),
        vec![evening_lock().into(), unlock_at(at(5, 18, 30))],
        vec![],
    );

    assert_eq!(result.timeline().provisional_work(), minutes(104));
    assert!(
        result
            .timeline()
            .segments()
            .iter()
            .all(|segment| segment.label() != AccountingLabel::OvernightLock)
    );
    let [review] = result.pending_reviews() else {
        panic!("expected one review");
    };
    assert_eq!(
        review.locked_episode,
        interval(at(5, 16, 46), at(5, 18, 30))
    );
    assert!(!review.is_open);
}

#[test]
fn lock_followed_by_suspend_before_midnight_splits_into_overnight_lock_and_suspended() {
    let suspend = Recorded::new(
        WorkdayEventId(356),
        at(5, 19, 0),
        PowerState::Suspended,
        origin("windows:power"),
    );

    let result = calculate_monday(
        at(6, 10, 0),
        vec![evening_lock().into(), suspend.into()],
        vec![],
    );

    let after_lock: Vec<_> = result
        .timeline()
        .segments()
        .iter()
        .filter(|segment| segment.interval().start() >= at(5, 16, 46))
        .map(shape)
        .collect();
    assert_eq!(
        after_lock,
        [
            (
                Type::Excluded,
                interval(at(5, 16, 46), at(5, 19, 0)),
                AccountingLabel::OvernightLock
            ),
            (
                Type::Excluded,
                interval(at(5, 19, 0), at(6, 0, 0)),
                AccountingLabel::Suspended
            ),
        ]
    );
    assert_eq!(result.timeline().provisional_work(), SignedDuration::ZERO);
    assert!(result.pending_reviews().is_empty());
}

#[test]
fn declaring_an_overnight_lock_as_work_credits_it() {
    let overnight = interval(at(5, 16, 46), at(6, 0, 0));
    let declaration = ManualAdjustment::interval_work(
        AdjustmentId::new("declare-overnight").unwrap(),
        monday(),
        overnight,
        at(6, 10, 0),
    )
    .unwrap();

    let current = calculate_monday(at(6, 10, 0), vec![evening_lock().into()], vec![]);
    validate_for_append(&current, &[], &declaration).unwrap();
    let result = calculate_monday(
        at(6, 10, 0),
        vec![evening_lock().into()],
        vec![declaration.clone()],
    );

    let declared: Vec<_> = result
        .timeline()
        .segments()
        .iter()
        .filter(|segment| segment.interval() == overnight)
        .collect();
    let [segment] = declared.as_slice() else {
        panic!("expected one declared segment");
    };
    assert_eq!(segment.kind(), &SegmentKind::Credited);
    assert_eq!(segment.label(), AccountingLabel::Manual);
    assert_eq!(manual_id(segment), declaration.id().as_str());
    assert_eq!(result.credited_work(), minutes(526 + 434));
}

#[test]
fn persisted_confirmation_of_a_former_crossed_workday_review_still_replays() {
    let confirmation = ManualAdjustment::long_lock_confirmation(
        AdjustmentId::new("confirm-cross-workday").unwrap(),
        tuesday(),
        interval(at(6, 0, 0), at(6, 6, 57)),
        at(6, 7, 7),
        lock_review_id(),
    )
    .unwrap();

    let result = calculate_tuesday(
        at(6, 7, 7),
        vec![unlock_at(at(6, 6, 57))],
        vec![confirmation.clone()],
    );

    assert!(result.pending_reviews().is_empty());
    assert_eq!(result.credited_work(), minutes(427));
    let segment = &result.timeline().segments()[0];
    assert_eq!(segment.kind(), &SegmentKind::Credited);
    assert_eq!(manual_id(segment), confirmation.id().as_str());
}
