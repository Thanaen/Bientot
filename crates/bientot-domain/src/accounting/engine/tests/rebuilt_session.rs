//! The time a session was open before Bientôt watched it, rebuilt from the Windows logs
//! (issue #75).

use jiff::Timestamp;
use jiff::civil::date;
use jiff::tz::TimeZone;

use super::{Type, minutes, origin, shapes};
use crate::accounting::{
    AccountingInput, AccountingLabel, AccountingPolicy, AccountingResult, ActivityCoverage,
    ActivitySegment, ActivityState, AdjustmentId, LongLockReviewId, LongLockReviewReason,
    PowerState, Recorded, RecordedTransition, SESSION_HISTORY_SOURCE_ID, SessionState,
    WorkdayEventHistory, WorkdayEventId, WorkdayStateAnchors, calculate,
};
use crate::adjustments::{ManualAdjustment, ManualClassification, validate_for_append};
use crate::intervals::UtcInterval;
use crate::time::{LocalWorkday, WorkdayWindow};

fn at(hour: i8, minute: i8) -> Timestamp {
    date(2026, 10, 26)
        .at(hour, minute, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn between(start: (i8, i8), end: (i8, i8)) -> UtcInterval {
    UtcInterval::new(at(start.0, start.1), at(end.0, end.1)).unwrap()
}

fn session(id: i64, at: (i8, i8), state: SessionState, source: &str) -> RecordedTransition {
    Recorded::new(
        WorkdayEventId(id),
        self::at(at.0, at.1),
        state,
        origin(source),
    )
    .into()
}

/// Read from the logs.
fn rebuilt(id: i64, at: (i8, i8), state: SessionState) -> RecordedTransition {
    session(id, at, state, SESSION_HISTORY_SOURCE_ID)
}

/// Observed by Bientôt as it happened.
fn observed(id: i64, at: (i8, i8), state: SessionState) -> RecordedTransition {
    session(id, at, state, "windows-session")
}

fn review(unlock: i64, start: (i8, i8)) -> LongLockReviewId {
    LongLockReviewId::for_rebuilt_session(WorkdayEventId(unlock), at(start.0, start.1))
}

/// Installed at 14:00 on a Monday from 00:00 to 23:59, with lunch from 12:00 to 13:00: the logs
/// tell an unlock at 09:00, a lock from 10:00 to 10:20, and Bientôt watches from 14:00 on.
struct Day {
    transitions: Vec<RecordedTransition>,
    adjustments: Vec<ManualAdjustment>,
}

fn installed_at_two() -> Day {
    Day {
        transitions: vec![
            rebuilt(1, (9, 0), SessionState::Unlocked),
            rebuilt(2, (10, 0), SessionState::Locked),
            rebuilt(3, (10, 20), SessionState::Unlocked),
            observed(4, (14, 0), SessionState::Unlocked),
        ],
        adjustments: Vec::new(),
    }
}

impl Day {
    fn input(&self) -> AccountingInput {
        let window = WorkdayWindow {
            workday: LocalWorkday::new(date(2026, 10, 26), "UTC"),
            start: at(0, 0),
            end: at(23, 59),
        };
        let policy = AccountingPolicy::new(between((12, 0), (13, 0)), minutes(90)).unwrap();
        let watched = ActivitySegment::new(
            between((14, 0), (15, 0)),
            ActivityState::Active,
            origin("built-in"),
        );
        AccountingInput::new(
            window,
            at(15, 0),
            policy,
            ActivityCoverage::new(between((0, 0), (15, 0)), [watched]),
            WorkdayEventHistory::new(WorkdayStateAnchors::default(), self.transitions.clone()),
            self.adjustments.clone(),
        )
        .unwrap()
    }

    fn calculate(&self) -> AccountingResult {
        calculate(&self.input()).unwrap()
    }
}

#[test]
fn the_unlocked_morning_counts_provisionally_until_bientot_watches() {
    let result = installed_at_two().calculate();

    assert_eq!(
        shapes(&result),
        [
            (
                Type::Unknown,
                between((0, 0), (9, 0)),
                AccountingLabel::Unknown
            ),
            (
                Type::Provisional,
                between((9, 0), (10, 0)),
                AccountingLabel::RebuiltWork
            ),
            (
                Type::Provisional,
                between((10, 0), (10, 20)),
                AccountingLabel::LockedWork
            ),
            (
                Type::Provisional,
                between((10, 20), (12, 0)),
                AccountingLabel::RebuiltWork
            ),
            (
                Type::Unknown,
                between((12, 0), (13, 0)),
                AccountingLabel::Unknown
            ),
            (
                Type::Provisional,
                between((13, 0), (14, 0)),
                AccountingLabel::RebuiltWork
            ),
            (
                Type::Credited,
                between((14, 0), (15, 0)),
                AccountingLabel::ActiveWork
            ),
        ]
    );
}

#[test]
fn each_run_has_its_own_review_so_that_an_answer_leaves_lunch_alone() {
    let result = installed_at_two().calculate();

    let reviews: Vec<_> = result
        .pending_reviews()
        .iter()
        .map(|review| (review.id.clone(), review.reason, review.locked_episode))
        .collect();
    assert_eq!(
        reviews,
        [
            (
                review(1, (9, 0)),
                LongLockReviewReason::RebuiltSession,
                between((9, 0), (10, 0))
            ),
            (
                review(3, (10, 20)),
                LongLockReviewReason::RebuiltSession,
                between((10, 20), (12, 0))
            ),
            (
                review(3, (13, 0)),
                LongLockReviewReason::RebuiltSession,
                between((13, 0), (14, 0))
            ),
        ]
    );
    assert!(
        result
            .pending_reviews()
            .iter()
            .all(|review| !review.is_open)
    );
    assert_eq!(result.work_awaiting_review(), minutes(60 + 100 + 60));
}

#[test]
fn an_answer_settles_its_review() {
    let mut day = installed_at_two();
    let exclude = ManualAdjustment::reclassification(
        AdjustmentId::new("exclude-first").unwrap(),
        LocalWorkday::new(date(2026, 10, 26), "UTC"),
        between((9, 0), (10, 0)),
        ManualClassification::Excluded,
        at(15, 0),
    )
    .unwrap();
    validate_for_append(&day.calculate(), &day.adjustments, &exclude).unwrap();
    day.adjustments.push(exclude);

    let result = day.calculate();

    let pending: Vec<_> = result
        .pending_reviews()
        .iter()
        .map(|review| review.id.clone())
        .collect();
    assert_eq!(pending, [review(3, (10, 20)), review(3, (13, 0))]);
}

#[test]
fn an_unlock_bientot_observed_or_a_suspend_is_not_rebuilt() {
    let mut day = installed_at_two();
    day.transitions = vec![
        observed(1, (9, 0), SessionState::Unlocked),
        rebuilt(2, (10, 0), SessionState::Unlocked),
        Recorded::new(
            WorkdayEventId(3),
            at(11, 0),
            PowerState::Suspended,
            origin("windows-power"),
        )
        .into(),
        Recorded::new(
            WorkdayEventId(4),
            at(11, 30),
            PowerState::Active,
            origin("windows-power"),
        )
        .into(),
        observed(5, (14, 0), SessionState::Unlocked),
    ];

    let rebuilt: Vec<_> = shapes(&day.calculate())
        .into_iter()
        .filter(|(_, _, label)| *label == AccountingLabel::RebuiltWork)
        .map(|(_, interval, _)| interval)
        .collect();

    assert_eq!(
        rebuilt,
        [
            between((10, 0), (11, 0)),
            between((11, 30), (12, 0)),
            between((13, 0), (14, 0))
        ]
    );
}
