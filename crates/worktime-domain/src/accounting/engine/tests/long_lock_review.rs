use jiff::civil::date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use super::{Type, hours, manual_id, minutes, origin, shape, single, single_labelled};
use crate::accounting::{
    AccountedOrigin, AccountingInput, AccountingLabel, AccountingPolicy, AccountingResult,
    ActivityCoverage, ActivitySegment, ActivityState, AdjustmentId, LongLockReviewId,
    LongLockReviewReason, PowerState, Recorded, SegmentKind, SessionState, WorkdayEventHistory,
    WorkdayEventId, WorkdayStateAnchors, calculate,
};
use crate::adjustments::{AdjustmentValidationError, ManualAdjustment, validate_for_append};
use crate::intervals::UtcInterval;
use crate::time::{LocalWorkday, WorkdayWindow};

fn at(hour: i8, minute: i8) -> Timestamp {
    date(2026, 8, 3)
        .at(hour, minute, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn between(start_hour: i8, start_minute: i8, end_hour: i8, end_minute: i8) -> UtcInterval {
    UtcInterval::new(at(start_hour, start_minute), at(end_hour, end_minute)).unwrap()
}

fn workday() -> LocalWorkday {
    LocalWorkday::new(date(2026, 8, 3), "UTC")
}

fn review_id() -> LongLockReviewId {
    LongLockReviewId::new("session-lock:1").unwrap()
}

fn session(id: i64, occurred_at: Timestamp, state: SessionState) -> Recorded<SessionState> {
    Recorded::new(
        WorkdayEventId(id),
        occurred_at,
        state,
        origin("windows:session"),
    )
}

fn lock_between(locked_at: Timestamp, unlocked_at: Timestamp) -> WorkdayEventHistory {
    WorkdayEventHistory::new(
        WorkdayStateAnchors::default(),
        [
            session(1, locked_at, SessionState::Locked).into(),
            session(2, unlocked_at, SessionState::Unlocked).into(),
        ],
    )
}

fn confirmation(interval: UtcInterval) -> ManualAdjustment {
    ManualAdjustment::long_lock_confirmation(
        AdjustmentId::new("confirm-lock-1").unwrap(),
        workday(),
        interval,
        interval.end() + minutes(1),
        review_id(),
    )
    .unwrap()
}

struct Scenario {
    through: Timestamp,
    lock_at: Option<Timestamp>,
    window_start: Option<Timestamp>,
    adjustments: Vec<ManualAdjustment>,
    review_threshold: SignedDuration,
    event_history: Option<WorkdayEventHistory>,
    activity_state: ActivityState,
}

fn through(through: Timestamp) -> Scenario {
    Scenario {
        through,
        lock_at: None,
        window_start: None,
        adjustments: Vec::new(),
        review_threshold: minutes(90),
        event_history: None,
        activity_state: ActivityState::Active,
    }
}

impl Scenario {
    fn calculate(self) -> AccountingResult {
        let start = self.window_start.unwrap_or(at(8, 0));
        let end = at(15, 0);
        let bounds = UtcInterval::new(start, end).unwrap();
        let history = self.event_history.unwrap_or_else(|| {
            WorkdayEventHistory::new(
                WorkdayStateAnchors::default(),
                [session(1, self.lock_at.unwrap_or(start), SessionState::Locked).into()],
            )
        });
        let input = AccountingInput::new(
            WorkdayWindow {
                workday: workday(),
                start,
                end,
            },
            self.through,
            AccountingPolicy::new(between(12, 0, 14, 0), self.review_threshold).unwrap(),
            ActivityCoverage::new(
                bounds,
                [ActivitySegment::new(
                    bounds,
                    self.activity_state,
                    origin("activitywatch:afk"),
                )],
            ),
            history,
            self.adjustments,
        )
        .unwrap();
        calculate(&input).unwrap()
    }
}

#[test]
fn lock_below_threshold_remains_provisional_without_pending_review() {
    let result = through(at(9, 29)).calculate();

    assert_eq!(result.timeline().provisional_work(), minutes(89));
    assert!(result.pending_reviews().is_empty());
    assert_eq!(result.work_awaiting_review(), SignedDuration::ZERO);
}

#[test]
fn lock_at_threshold_creates_one_pending_review() {
    let result = through(at(9, 30)).calculate();

    let [review] = result.pending_reviews() else {
        panic!("expected one review");
    };
    assert_eq!(review.id, review_id());
    assert_eq!(review.locked_episode.duration(), minutes(90));
    assert_eq!(review.provisional_coverage.covered_duration(), minutes(90));
    assert_eq!(review.reason, LongLockReviewReason::ThresholdExceeded);
    assert!(review.is_open);
    assert_eq!(result.work_awaiting_review(), minutes(90));
}

#[test]
fn unlock_closes_the_same_pending_review_at_a_fixed_episode_end() {
    let open = through(at(9, 30)).calculate();

    let closed = Scenario {
        event_history: Some(lock_between(at(8, 0), at(9, 30))),
        ..through(at(10, 0))
    }
    .calculate();

    let ([open_review], [closed_review]) = (open.pending_reviews(), closed.pending_reviews())
    else {
        panic!("expected one review on each side");
    };
    assert_eq!(open_review.id, closed_review.id);
    assert_eq!(open_review.locked_episode, closed_review.locked_episode);
    assert!(open_review.is_open);
    assert!(!closed_review.is_open);
    assert_eq!(closed.work_awaiting_review(), minutes(90));
}

#[test]
fn lunch_coverage_is_excluded_from_the_pending_review_coverage() {
    let result = Scenario {
        lock_at: Some(at(11, 30)),
        window_start: Some(at(11, 30)),
        ..through(at(14, 30))
    }
    .calculate();

    let [review] = result.pending_reviews() else {
        panic!("expected one review");
    };
    assert_eq!(review.provisional_coverage.covered_duration(), hours(1));
    assert_eq!(result.work_awaiting_review(), hours(1));
    assert_eq!(result.timeline().excluded_work(), hours(2));
}

#[test]
fn confirming_long_lock_as_work_replaces_provisional_credit_and_clears_review() {
    let confirmation = ManualAdjustment::long_lock_confirmation(
        AdjustmentId::new("confirm-lock-1").unwrap(),
        workday(),
        between(8, 0, 9, 30),
        at(9, 31),
        review_id(),
    )
    .unwrap();

    let current = through(at(9, 30)).calculate();
    validate_for_append(&current, &[], &confirmation).unwrap();
    let result = Scenario {
        adjustments: vec![confirmation.clone()],
        ..through(at(9, 30))
    }
    .calculate();

    assert!(result.pending_reviews().is_empty());
    assert_eq!(result.credited_work(), minutes(90));
    assert_eq!(result.timeline().provisional_work(), SignedDuration::ZERO);
    let segment = single(&result);
    assert_eq!(segment.kind(), &SegmentKind::Credited);
    assert_eq!(segment.label(), AccountingLabel::Manual);
    assert_eq!(manual_id(segment), confirmation.id().as_str());
}

#[test]
fn confirmation_command_requires_a_pending_review() {
    let confirmation = confirmation(between(8, 0, 9, 30));
    let current = Scenario {
        review_threshold: hours(4),
        ..through(at(9, 30))
    }
    .calculate();

    let error = validate_for_append(&current, &[], &confirmation).unwrap_err();

    assert!(matches!(
        error,
        AdjustmentValidationError::UnknownLongLockReview { .. }
    ));
}

#[test]
fn confirmation_command_requires_the_complete_locked_episode() {
    let confirmation = confirmation(between(8, 0, 9, 0));
    let current = through(at(9, 30)).calculate();

    let error = validate_for_append(&current, &[], &confirmation).unwrap_err();

    assert!(matches!(
        error,
        AdjustmentValidationError::IncompleteLongLockConfirmation { .. }
    ));
}

#[test]
fn persisted_confirmation_across_lunch_replays_as_recorded() {
    let confirmation = ManualAdjustment::long_lock_confirmation(
        AdjustmentId::new("confirm-lock-1").unwrap(),
        workday(),
        between(11, 30, 14, 30),
        at(14, 31),
        review_id(),
    )
    .unwrap();

    let result = Scenario {
        lock_at: Some(at(11, 30)),
        window_start: Some(at(11, 30)),
        adjustments: vec![confirmation.clone()],
        ..through(at(14, 30))
    }
    .calculate();

    assert!(result.pending_reviews().is_empty());
    assert_eq!(result.credited_work(), hours(3));
    assert_eq!(result.timeline().excluded_work(), SignedDuration::ZERO);
    let segment = single(&result);
    assert_eq!(segment.kind(), &SegmentKind::Credited);
    assert_eq!(manual_id(segment), confirmation.id().as_str());
}

#[test]
fn a_lock_that_runs_into_lunch_shortly_before_it_starts_is_lunch() {
    let locked_at = at(11, 50) + SignedDuration::from_secs(22);
    let unlocked_at = at(13, 29) + SignedDuration::from_secs(3);

    let result = Scenario {
        review_threshold: minutes(45),
        event_history: Some(lock_between(locked_at, unlocked_at)),
        ..through(at(14, 30))
    }
    .calculate();

    assert!(result.pending_reviews().is_empty());
    assert_eq!(result.timeline().provisional_work(), SignedDuration::ZERO);
    let lunch = single_labelled(&result, AccountingLabel::LunchLock);
    assert_eq!(
        shape(lunch),
        (
            Type::Excluded,
            UtcInterval::new(locked_at, unlocked_at).unwrap(),
            AccountingLabel::LunchLock
        )
    );
    assert_eq!(
        result.credited_work(),
        hours(4) + minutes(51) + SignedDuration::from_secs(19)
    );
}

#[test]
fn a_late_return_from_lunch_is_lunch() {
    let result = Scenario {
        review_threshold: minutes(45),
        event_history: Some(lock_between(at(12, 30), at(14, 20))),
        ..through(at(15, 0))
    }
    .calculate();

    assert!(result.pending_reviews().is_empty());
    assert_eq!(result.timeline().provisional_work(), SignedDuration::ZERO);
    assert_eq!(
        single_labelled(&result, AccountingLabel::LunchLock).interval(),
        between(12, 30, 14, 20)
    );
}

#[test]
fn an_open_lock_that_ran_into_lunch_is_lunch_while_it_lasts() {
    let result = Scenario {
        lock_at: Some(at(11, 50)),
        review_threshold: minutes(45),
        ..through(at(12, 30))
    }
    .calculate();

    assert!(result.pending_reviews().is_empty());
    assert_eq!(result.timeline().provisional_work(), SignedDuration::ZERO);
    assert_eq!(
        single_labelled(&result, AccountingLabel::LunchLock).interval(),
        between(11, 50, 12, 30)
    );
}

fn provisional_intervals(result: &AccountingResult) -> Vec<UtcInterval> {
    result
        .timeline()
        .segments()
        .iter()
        .filter(|segment| matches!(segment.kind(), SegmentKind::ProvisionalCredit { .. }))
        .map(|segment| segment.interval())
        .collect()
}

#[test]
fn a_lock_reaching_the_threshold_before_lunch_is_still_reviewed() {
    let result = Scenario {
        review_threshold: minutes(45),
        event_history: Some(lock_between(at(11, 15), at(12, 30))),
        ..through(at(15, 0))
    }
    .calculate();

    let [review] = result.pending_reviews() else {
        panic!("expected one review");
    };
    assert_eq!(review.provisional_coverage.covered_duration(), minutes(45));
    assert_eq!(provisional_intervals(&result), [between(11, 15, 12, 0)]);
}

#[test]
fn a_lock_reaching_the_threshold_after_lunch_is_still_reviewed() {
    let result = Scenario {
        review_threshold: minutes(45),
        event_history: Some(lock_between(at(13, 0), at(14, 50))),
        ..through(at(15, 0))
    }
    .calculate();

    let [review] = result.pending_reviews() else {
        panic!("expected one review");
    };
    assert_eq!(review.provisional_coverage.covered_duration(), minutes(50));
}

#[test]
fn a_lock_spanning_the_whole_lunch_is_still_reviewed() {
    let result = Scenario {
        review_threshold: minutes(45),
        event_history: Some(lock_between(at(11, 50), at(14, 10))),
        ..through(at(15, 0))
    }
    .calculate();

    let [review] = result.pending_reviews() else {
        panic!("expected one review");
    };
    assert_eq!(review.provisional_coverage.covered_duration(), minutes(20));
}

#[test]
fn a_short_lock_ending_before_lunch_keeps_its_provisional_credit() {
    let result = Scenario {
        review_threshold: minutes(45),
        event_history: Some(lock_between(at(11, 20), at(11, 50))),
        ..through(at(15, 0))
    }
    .calculate();

    assert!(result.pending_reviews().is_empty());
    let provisional: Vec<_> = result
        .timeline()
        .segments()
        .iter()
        .filter(|segment| matches!(segment.kind(), SegmentKind::ProvisionalCredit { .. }))
        .map(shape)
        .collect();
    assert_eq!(
        provisional,
        [(
            Type::Provisional,
            between(11, 20, 11, 50),
            AccountingLabel::LockedWork
        )]
    );
}

#[test]
fn persisted_confirmation_replays_after_review_threshold_changes() {
    let confirmation = confirmation(between(8, 0, 9, 30));

    let result = Scenario {
        adjustments: vec![confirmation.clone()],
        review_threshold: hours(4),
        ..through(at(9, 30))
    }
    .calculate();

    assert!(result.pending_reviews().is_empty());
    let segment = single(&result);
    assert_eq!(segment.kind(), &SegmentKind::Credited);
    assert_eq!(manual_id(segment), confirmation.id().as_str());
}

#[test]
fn persisted_confirmation_replays_after_source_classification_changes() {
    let confirmation = confirmation(between(8, 0, 9, 30));

    let result = Scenario {
        adjustments: vec![confirmation.clone()],
        event_history: Some(WorkdayEventHistory::default()),
        activity_state: ActivityState::Afk,
        ..through(at(9, 30))
    }
    .calculate();

    assert!(result.pending_reviews().is_empty());
    let segment = single(&result);
    assert_eq!(segment.kind(), &SegmentKind::Credited);
    assert_eq!(manual_id(segment), confirmation.id().as_str());
    assert!(matches!(
        segment.origin(),
        AccountedOrigin::Manual { replaces, .. } if matches!(**replaces, AccountedOrigin::Automatic(_))
    ));
}

#[test]
fn persisted_confirmation_preserves_suspend_after_source_changes() {
    let confirmation = confirmation(between(8, 0, 9, 30));
    let suspended_history = WorkdayEventHistory::new(
        WorkdayStateAnchors {
            power: Some(Recorded::new(
                WorkdayEventId(2),
                at(7, 59),
                PowerState::Suspended,
                origin("windows:power"),
            )),
            ..WorkdayStateAnchors::default()
        },
        [],
    );

    let result = Scenario {
        adjustments: vec![confirmation],
        event_history: Some(suspended_history),
        ..through(at(9, 30))
    }
    .calculate();

    assert!(result.pending_reviews().is_empty());
    assert_eq!(result.credited_work(), SignedDuration::ZERO);
    let segment = single(&result);
    assert_eq!(segment.kind(), &SegmentKind::Excluded);
    assert_eq!(segment.label(), AccountingLabel::Suspended);
    assert!(matches!(segment.origin(), AccountedOrigin::Automatic(_)));
}

#[test]
fn lock_carried_from_an_earlier_workday_has_no_review_to_confirm() {
    let history = WorkdayEventHistory::new(
        WorkdayStateAnchors {
            session: Some(session(1, at(7, 0), SessionState::Locked)),
            ..WorkdayStateAnchors::default()
        },
        [],
    );
    let current = Scenario {
        event_history: Some(history),
        ..through(at(9, 0))
    }
    .calculate();
    let confirmation = confirmation(between(8, 0, 9, 0));

    let error = validate_for_append(&current, &[], &confirmation).unwrap_err();

    assert!(matches!(
        error,
        AdjustmentValidationError::UnknownLongLockReview { .. }
    ));
    assert!(current.pending_reviews().is_empty());
    let segment = single(&current);
    assert_eq!(segment.kind(), &SegmentKind::Excluded);
    assert_eq!(segment.label(), AccountingLabel::OvernightLock);
}

#[test]
fn long_lock_confirmation_may_span_suspend_and_replay_keeps_suspend_excluded() {
    let power = |id, occurred_at, state| {
        Recorded::new(
            WorkdayEventId(id),
            occurred_at,
            state,
            origin("windows:power"),
        )
        .into()
    };
    let history = WorkdayEventHistory::new(
        WorkdayStateAnchors::default(),
        [
            session(1, at(8, 0), SessionState::Locked).into(),
            power(2, at(8, 30), PowerState::Suspended),
            power(3, at(9, 0), PowerState::Active),
        ],
    );
    let current = Scenario {
        event_history: Some(history.clone()),
        ..through(at(9, 30))
    }
    .calculate();
    let confirmation = confirmation(between(8, 0, 9, 30));

    validate_for_append(&current, &[], &confirmation).unwrap();
    let replayed = Scenario {
        adjustments: vec![confirmation],
        event_history: Some(history),
        ..through(at(9, 30))
    }
    .calculate();

    assert!(replayed.pending_reviews().is_empty());
    assert_eq!(current.work_awaiting_review(), hours(1));
    assert_eq!(replayed.work_awaiting_review(), SignedDuration::ZERO);
    assert_eq!(replayed.timeline().excluded_work(), minutes(30));
    let suspended = single_labelled(&replayed, AccountingLabel::Suspended);
    assert_eq!(suspended.interval(), between(8, 30, 9, 0));
    assert!(matches!(suspended.origin(), AccountedOrigin::Automatic(_)));
}

#[test]
fn a_repeated_lock_keeps_the_review_of_the_first_lock() {
    let history = WorkdayEventHistory::new(
        WorkdayStateAnchors::default(),
        [
            session(1, at(8, 0), SessionState::Locked).into(),
            session(2, at(8, 30), SessionState::Locked).into(),
        ],
    );

    let result = Scenario {
        event_history: Some(history),
        ..through(at(9, 30))
    }
    .calculate();

    let [review] = result.pending_reviews() else {
        panic!("expected one review");
    };
    assert_eq!(review.id, review_id());
    assert_eq!(review.provisional_coverage.covered_duration(), minutes(90));
}
