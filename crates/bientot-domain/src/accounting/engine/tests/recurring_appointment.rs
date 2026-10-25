//! Absences during a recurring appointment (issue #321).

use jiff::Timestamp;
use jiff::civil::date;
use jiff::tz::TimeZone;

use super::{Type, minutes, origin, shape};
use crate::accounting::{
    AccountingInput, AccountingLabel, AccountingPolicy, AccountingResult, ActivityCoverage,
    ActivitySegment, ActivityState, AdjustmentId, AppointmentWindow, LongLockReviewId,
    LongLockReviewReason, PowerState, Recorded, RecordedTransition, SessionState, TrackingState,
    WorkdayEventHistory, WorkdayEventId, WorkdayStateAnchors, calculate,
};
use crate::adjustments::{ManualAdjustment, ManualClassification, validate_for_append};
use crate::intervals::UtcInterval;
use crate::time::{LocalWorkday, WorkdayWindow};

fn at(hour: i8, minute: i8) -> Timestamp {
    date(2026, 10, 22)
        .at(hour, minute, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn between(start: (i8, i8), end: (i8, i8)) -> UtcInterval {
    UtcInterval::new(at(start.0, start.1), at(end.0, end.1)).unwrap()
}

fn workday() -> LocalWorkday {
    LocalWorkday::new(date(2026, 10, 22), "UTC")
}

fn review_from(start: (i8, i8)) -> LongLockReviewId {
    LongLockReviewId::for_appointment("meeting", at(start.0, start.1))
}

/// A Thursday from 08:00 to 20:00 with a meeting from 14:00 to 16:00, accounted through `through`.
struct Day {
    through: Timestamp,
    activity: Vec<(UtcInterval, ActivityState)>,
    transitions: Vec<RecordedTransition>,
    adjustments: Vec<ManualAdjustment>,
}

/// At the PC from 09:00, away from `away.0` to `away.1`, back at the PC until 18:00.
fn away(start: (i8, i8), end: (i8, i8)) -> Day {
    Day {
        through: at(18, 0),
        activity: vec![
            (between((9, 0), start), ActivityState::Active),
            (between(start, end), ActivityState::Afk),
            (between(end, (18, 0)), ActivityState::Active),
        ],
        transitions: Vec::new(),
        adjustments: Vec::new(),
    }
}

impl Day {
    fn input(&self) -> AccountingInput {
        let window = WorkdayWindow {
            workday: workday(),
            start: at(8, 0),
            end: at(20, 0),
        };
        let policy = AccountingPolicy::new(between((12, 0), (13, 0)), minutes(90))
            .unwrap()
            .with_appointments(vec![AppointmentWindow {
                appointment_id: "meeting".to_owned(),
                interval: between((14, 0), (16, 0)),
            }]);
        let segments = self.activity.iter().map(|&(interval, state)| {
            ActivitySegment::new(interval, state, origin("built-in:idle"))
        });
        AccountingInput::new(
            window,
            self.through,
            policy,
            ActivityCoverage::new(between((8, 0), (18, 0)), segments),
            WorkdayEventHistory::new(WorkdayStateAnchors::default(), self.transitions.clone()),
            self.adjustments.clone(),
        )
        .unwrap()
    }

    fn calculate(&self) -> AccountingResult {
        calculate(&self.input()).unwrap()
    }

    fn with(mut self, adjustment: ManualAdjustment) -> Self {
        validate_for_append(&self.calculate(), &self.adjustments, &adjustment).unwrap();
        self.adjustments.push(adjustment);
        self
    }
}

fn scheduled(result: &AccountingResult) -> Vec<(Type, UtcInterval)> {
    result
        .timeline()
        .segments()
        .iter()
        .map(shape)
        .filter(|(_, _, label)| *label == AccountingLabel::ScheduledAway)
        .map(|(kind, interval, _)| (kind, interval))
        .collect()
}

#[test]
fn an_absence_during_the_appointment_is_credited_until_reviewed() {
    let result = away((14, 2), (15, 47)).calculate();

    assert_eq!(
        scheduled(&result),
        [(Type::Provisional, between((14, 2), (15, 47)))]
    );
    let [review] = result.pending_reviews() else {
        panic!("expected one review, got {:?}", result.pending_reviews());
    };
    assert_eq!(review.id, review_from((14, 2)));
    assert_eq!(review.reason, LongLockReviewReason::RecurringAppointment);
    assert_eq!(review.locked_episode, between((14, 2), (15, 47)));
    assert!(!review.is_open);
    assert_eq!(result.work_awaiting_review(), minutes(105));
}

#[test]
fn the_absence_counts_from_a_quarter_hour_before_to_a_quarter_hour_after() {
    let result = away((13, 30), (16, 40)).calculate();

    assert_eq!(
        scheduled(&result),
        [(Type::Provisional, between((13, 45), (16, 15)))]
    );
    let afk: Vec<_> = result
        .timeline()
        .segments()
        .iter()
        .filter(|segment| segment.label() == AccountingLabel::Afk)
        .map(|segment| segment.interval())
        .collect();
    assert_eq!(
        afk,
        [between((13, 30), (13, 45)), between((16, 15), (16, 40))]
    );
}

#[test]
fn an_absence_outside_the_appointment_is_left_alone() {
    let result = away((16, 20), (17, 0)).calculate();

    assert!(scheduled(&result).is_empty());
    assert!(result.pending_reviews().is_empty());
}

#[test]
fn a_short_absence_or_a_day_not_yet_started_is_left_alone() {
    assert!(scheduled(&away((14, 30), (14, 34)).calculate()).is_empty());

    let mut late = away((14, 0), (15, 0));
    late.activity = vec![
        (between((8, 0), (15, 0)), ActivityState::Afk),
        (between((15, 0), (18, 0)), ActivityState::Active),
    ];
    let result = late.calculate();
    assert!(scheduled(&result).is_empty());
    assert!(result.pending_reviews().is_empty());
}

#[test]
fn each_absence_of_the_appointment_has_its_own_review_and_the_work_between_stays() {
    let mut day = away((14, 0), (16, 0));
    day.activity = vec![
        (between((9, 0), (14, 0)), ActivityState::Active),
        (between((14, 0), (14, 10)), ActivityState::Afk),
        (between((14, 10), (15, 40)), ActivityState::Active),
        (between((15, 40), (15, 50)), ActivityState::Afk),
        (between((15, 50), (18, 0)), ActivityState::Active),
    ];
    let reviews = |result: &AccountingResult| -> Vec<_> {
        result
            .pending_reviews()
            .iter()
            .map(|review| (review.id.clone(), review.locked_episode))
            .collect()
    };

    let result = day.calculate();
    assert_eq!(
        reviews(&result),
        [
            (review_from((14, 0)), between((14, 0), (14, 10))),
            (review_from((15, 40)), between((15, 40), (15, 50))),
        ]
    );

    let result = day
        .with(
            ManualAdjustment::reclassification(
                AdjustmentId::new("exclude-first").unwrap(),
                workday(),
                between((14, 0), (14, 10)),
                ManualClassification::Excluded,
                at(18, 0),
            )
            .unwrap(),
        )
        .calculate();
    assert_eq!(
        reviews(&result),
        [(review_from((15, 40)), between((15, 40), (15, 50)))]
    );
    assert_eq!(result.credited_work(), minutes(9 * 60 - 10));
}

#[test]
fn an_absence_after_an_answer_opens_a_new_review() {
    let mut day = away((14, 0), (16, 0));
    day.activity = vec![
        (between((9, 0), (14, 0)), ActivityState::Active),
        (between((14, 0), (14, 20)), ActivityState::Afk),
        (between((14, 20), (15, 0)), ActivityState::Active),
        (between((15, 0), (15, 30)), ActivityState::Afk),
        (between((15, 30), (18, 0)), ActivityState::Active),
    ];
    day.through = at(14, 30);
    let day = day.with(
        ManualAdjustment::long_lock_confirmation(
            AdjustmentId::new("confirm-first").unwrap(),
            workday(),
            between((14, 0), (14, 20)),
            at(14, 30),
            review_from((14, 0)),
        )
        .unwrap(),
    );
    let mut later = day;
    later.through = at(18, 0);

    let result = later.calculate();

    let [review] = result.pending_reviews() else {
        panic!("expected one review, got {:?}", result.pending_reviews());
    };
    assert_eq!(review.id, review_from((15, 0)));
    assert_eq!(review.locked_episode, between((15, 0), (15, 30)));
}

#[test]
fn sleep_while_locked_and_lunch_stay_out_of_the_appointment() {
    let mut locked = away((14, 0), (14, 1));
    locked.activity = vec![
        (between((9, 0), (13, 50)), ActivityState::Active),
        (between((13, 50), (15, 40)), ActivityState::Afk),
        (between((15, 40), (18, 0)), ActivityState::Active),
    ];
    locked.transitions = vec![
        Recorded::new(
            WorkdayEventId(1),
            at(13, 50),
            SessionState::Locked,
            origin("windows:session"),
        )
        .into(),
        Recorded::new(
            WorkdayEventId(2),
            at(14, 0),
            PowerState::Suspended,
            origin("windows:power"),
        )
        .into(),
        Recorded::new(
            WorkdayEventId(3),
            at(15, 0),
            PowerState::Active,
            origin("windows:power"),
        )
        .into(),
        Recorded::new(
            WorkdayEventId(4),
            at(15, 40),
            SessionState::Unlocked,
            origin("windows:session"),
        )
        .into(),
    ];
    let result = locked.calculate();
    assert!(
        result
            .pending_reviews()
            .iter()
            .all(|review| review.reason == LongLockReviewReason::ThresholdExceeded),
        "{:?}",
        result.pending_reviews()
    );
    assert!(
        scheduled(&result)
            .iter()
            .all(|(_, interval)| !interval.overlaps(between((13, 50), (15, 40))))
    );

    let mut lunch = away((11, 0), (11, 1));
    lunch.activity = vec![
        (between((9, 0), (11, 0)), ActivityState::Active),
        (between((11, 0), (14, 0)), ActivityState::Afk),
        (between((14, 0), (18, 0)), ActivityState::Active),
    ];
    let mut input = lunch.input();
    input = AccountingInput::new(
        input.window().clone(),
        input.now(),
        input
            .policy()
            .clone()
            .with_appointments(vec![AppointmentWindow {
                appointment_id: "meeting".to_owned(),
                interval: between((11, 30), (13, 30)),
            }]),
        input.activity().clone(),
        input.events().clone(),
        Vec::new(),
    )
    .unwrap();
    let result = calculate(&input).unwrap();
    assert_eq!(
        scheduled(&result),
        [
            (Type::Provisional, between((11, 15), (12, 0))),
            (Type::Provisional, between((13, 0), (13, 45))),
        ]
    );
}

#[test]
fn sleep_during_the_appointment_is_credited_but_not_while_paused() {
    let power = |id, time: (i8, i8), state| {
        Recorded::new(
            WorkdayEventId(id),
            at(time.0, time.1),
            state,
            origin("windows:power"),
        )
        .into()
    };
    let mut asleep = away((14, 0), (14, 10));
    asleep.transitions = vec![
        power(1, (14, 10), PowerState::Suspended),
        power(2, (15, 30), PowerState::Active),
    ];
    let result = asleep.calculate();
    assert_eq!(result.timeline().provisional_work(), minutes(90));
    let [review] = result.pending_reviews() else {
        panic!("expected one review");
    };
    assert_eq!(review.locked_episode, between((14, 0), (15, 30)));
    let credited = result.credited_work();
    let confirmed = asleep
        .with(
            ManualAdjustment::long_lock_confirmation(
                AdjustmentId::new("confirm").unwrap(),
                workday(),
                review.locked_episode,
                at(18, 0),
                review.id.clone(),
            )
            .unwrap(),
        )
        .calculate();
    assert_eq!(confirmed.credited_work(), credited);
    assert_eq!(confirmed.timeline().provisional_work(), minutes(0));

    let mut paused = away((14, 0), (14, 1));
    paused.transitions = vec![
        Recorded::new(
            WorkdayEventId(3),
            at(13, 55),
            TrackingState::Paused,
            origin("worktime:tracking"),
        )
        .into(),
        power(4, (14, 10), PowerState::Suspended),
        power(5, (15, 30), PowerState::Active),
    ];
    assert!(scheduled(&paused.calculate()).is_empty());
}

#[test]
fn an_absence_still_running_keeps_its_review_open() {
    let mut day = away((14, 2), (15, 0));
    day.activity.pop();
    day.through = at(15, 0);

    let result = day.calculate();

    let [review] = result.pending_reviews() else {
        panic!("expected one review");
    };
    assert_eq!(review.locked_episode, between((14, 2), (15, 0)));
    assert!(review.is_open);
}

#[test]
fn confirming_credits_the_absence_and_excluding_drops_it() {
    let episode = between((14, 2), (15, 47));
    let confirmed = away((14, 2), (15, 47)).with(
        ManualAdjustment::long_lock_confirmation(
            AdjustmentId::new("confirm").unwrap(),
            workday(),
            episode,
            at(18, 0),
            review_from((14, 2)),
        )
        .unwrap(),
    );
    let result = confirmed.calculate();
    assert!(result.pending_reviews().is_empty());
    assert_eq!(result.timeline().provisional_work(), minutes(0));
    assert_eq!(result.credited_work(), minutes(9 * 60));

    let excluded = away((14, 2), (15, 47)).with(
        ManualAdjustment::reclassification(
            AdjustmentId::new("exclude").unwrap(),
            workday(),
            episode,
            ManualClassification::Excluded,
            at(18, 0),
        )
        .unwrap(),
    );
    let result = excluded.calculate();
    assert!(result.pending_reviews().is_empty());
    assert_eq!(result.credited_work(), minutes(9 * 60 - 105));
}

#[test]
fn an_appointment_review_names_its_appointment() {
    assert_eq!(
        review_from((14, 2)).as_str(),
        "recurring-appointment:meeting:2026-10-22T14:02:00Z"
    );
    assert_eq!(review_from((14, 2)).appointment_id(), Some("meeting"));
    assert_eq!(
        LongLockReviewId::new("session-lock:4")
            .unwrap()
            .appointment_id(),
        None
    );
}

#[test]
fn an_absence_that_grows_back_over_an_answer_is_asked_only_for_the_rest() {
    let answered = away((13, 50), (14, 30)).with(
        ManualAdjustment::long_lock_confirmation(
            AdjustmentId::new("confirm").unwrap(),
            workday(),
            between((13, 50), (14, 30)),
            at(18, 0),
            review_from((13, 50)),
        )
        .unwrap(),
    );
    let mut revised = answered;
    revised.activity = vec![
        (between((9, 0), (13, 0)), ActivityState::Active),
        (between((13, 0), (14, 40)), ActivityState::Afk),
        (between((14, 40), (18, 0)), ActivityState::Active),
    ];

    let result = revised.calculate();

    let reviews: Vec<_> = result
        .pending_reviews()
        .iter()
        .map(|review| review.locked_episode)
        .collect();
    assert_eq!(reviews, [between((14, 30), (14, 40))]);
    let [review] = result.pending_reviews() else {
        unreachable!()
    };
    revised
        .with(
            ManualAdjustment::long_lock_confirmation(
                AdjustmentId::new("confirm-rest").unwrap(),
                workday(),
                review.locked_episode,
                at(18, 0),
                review.id.clone(),
            )
            .unwrap(),
        )
        .calculate();
}
