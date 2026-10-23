//! Properties of the accounting engine and manual adjustments that hold for any evidence, event
//! history and adjustment history.

use jiff::civil::date;
use jiff::{SignedDuration, Timestamp};
use proptest::prelude::*;
use worktime_domain::accounting::{
    AccountingInput, AccountingPolicy, AccountingResult, ActivityCoverage, ActivitySegment,
    ActivityState, AdjustmentId, AutomaticOrigin, PowerState, Recorded, RecordedTransition,
    SegmentKind, SessionState, TrackingState, WorkdayEventHistory, WorkdayEventId,
    WorkdayStateAnchors, calculate,
};
use worktime_domain::adjustments::{ManualAdjustment, ManualClassification};
use worktime_domain::intervals::{IntervalSet, UtcInterval};
use worktime_domain::time::{LocalWorkday, WorkdayWindow};

const WINDOW_MINUTES: i64 = 600;

fn minute(minute: i64) -> Timestamp {
    "2026-08-03T06:00:00Z".parse::<Timestamp>().unwrap() + SignedDuration::from_mins(minute)
}

fn interval(first: i64, second: i64) -> UtcInterval {
    UtcInterval::new(minute(first.min(second)), minute(first.max(second) + 1)).unwrap()
}

fn workday() -> LocalWorkday {
    LocalWorkday::new(date(2026, 8, 3), "UTC")
}

fn origin(id: &str) -> AutomaticOrigin {
    AutomaticOrigin::new(id).unwrap()
}

#[derive(Clone, Debug)]
struct Day {
    now: i64,
    lunch: (i64, i64),
    threshold: i64,
    activity: Vec<(i64, i64, usize, usize)>,
    anchors: (Option<usize>, Option<usize>, Option<usize>),
    transitions: Vec<(i64, usize, usize)>,
}

fn day() -> impl Strategy<Value = Day> {
    (
        -30i64..WINDOW_MINUTES + 30,
        (0i64..WINDOW_MINUTES, 0i64..WINDOW_MINUTES),
        1i64..180,
        prop::collection::vec(
            (
                -20i64..WINDOW_MINUTES + 20,
                -20i64..WINDOW_MINUTES + 20,
                0usize..3,
                0usize..2,
            ),
            0..10,
        ),
        (
            prop::option::of(0usize..3),
            prop::option::of(0usize..3),
            prop::option::of(0usize..3),
        ),
        prop::collection::vec((0i64..=WINDOW_MINUTES, 0usize..3, 0usize..3), 0..14),
    )
        .prop_map(
            |(now, lunch, threshold, activity, anchors, transitions)| Day {
                now,
                lunch,
                threshold,
                activity,
                anchors,
                transitions,
            },
        )
}

const SESSION: [SessionState; 3] = [
    SessionState::Unknown,
    SessionState::Locked,
    SessionState::Unlocked,
];
const POWER: [PowerState; 3] = [
    PowerState::Unknown,
    PowerState::Active,
    PowerState::Suspended,
];
const TRACKING: [TrackingState; 3] = [
    TrackingState::Running,
    TrackingState::Paused,
    TrackingState::Finished,
];

impl Day {
    fn window(&self) -> WorkdayWindow {
        WorkdayWindow {
            workday: workday(),
            start: minute(0),
            end: minute(WINDOW_MINUTES),
        }
    }

    fn through(&self) -> Timestamp {
        minute(self.now.clamp(0, WINDOW_MINUTES))
    }

    fn input(&self, adjustments: Vec<ManualAdjustment>) -> AccountingInput {
        let window = self.window();
        let states = [
            ActivityState::Active,
            ActivityState::Afk,
            ActivityState::Unknown,
        ];
        let sources = ["activitywatch:afk", "builtin:idle"];
        let activity = self.activity.iter().map(|&(a, b, state, source)| {
            ActivitySegment::new(interval(a, b), states[state], origin(sources[source]))
        });
        let before = minute(-60);
        let anchors = WorkdayStateAnchors {
            session: self.anchors.0.map(|state| {
                Recorded::new(
                    WorkdayEventId(-1),
                    before,
                    SESSION[state],
                    origin("session"),
                )
            }),
            power: self.anchors.1.map(|state| {
                Recorded::new(WorkdayEventId(-2), before, POWER[state], origin("power"))
            }),
            tracking: self.anchors.2.map(|state| {
                Recorded::new(
                    WorkdayEventId(-3),
                    before,
                    TRACKING[state],
                    origin("tracking"),
                )
            }),
        };
        let transitions = self.transitions.iter().enumerate().map(
            |(id, &(at, lane, state))| -> RecordedTransition {
                let (id, at) = (WorkdayEventId(id as i64), minute(at));
                match lane {
                    0 => Recorded::new(id, at, SESSION[state], origin("session")).into(),
                    1 => Recorded::new(id, at, POWER[state], origin("power")).into(),
                    _ => Recorded::new(id, at, TRACKING[state], origin("tracking")).into(),
                }
            },
        );
        AccountingInput::new(
            window.clone(),
            minute(self.now),
            AccountingPolicy::new(
                interval(self.lunch.0, self.lunch.1),
                SignedDuration::from_mins(self.threshold),
            )
            .unwrap(),
            ActivityCoverage::new(
                UtcInterval::new(window.start, window.end).unwrap(),
                activity,
            ),
            WorkdayEventHistory::new(anchors, transitions),
            adjustments,
        )
        .unwrap()
    }

    fn calculate(&self, adjustments: Vec<ManualAdjustment>) -> AccountingResult {
        calculate(&self.input(adjustments)).unwrap()
    }
}

/// Disjoint interval adjustments inside the horizon, built from sorted cut minutes.
fn interval_adjustments(day: &Day, cuts: &[(i64, bool)]) -> Vec<ManualAdjustment> {
    let horizon = day.now.clamp(0, WINDOW_MINUTES);
    let mut points: Vec<_> = cuts
        .iter()
        .map(|&(point, credited)| (point.rem_euclid(horizon.max(1)), credited))
        .collect();
    points.sort();
    points.dedup_by_key(|(point, _)| *point);
    points
        .chunks_exact(2)
        .enumerate()
        .map(|(index, pair)| {
            let span = UtcInterval::new(minute(pair[0].0), minute(pair[1].0)).unwrap();
            let id = AdjustmentId::new(format!("manual-{index}")).unwrap();
            let created_at = minute(WINDOW_MINUTES + index as i64);
            if pair[0].1 {
                ManualAdjustment::reclassification(
                    id,
                    workday(),
                    span,
                    ManualClassification::Credited,
                    created_at,
                )
            } else {
                ManualAdjustment::exclusion(id, workday(), span, created_at)
            }
            .unwrap()
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn the_timeline_covers_the_horizon_with_adjacent_canonical_segments(day in day()) {
        let result = day.calculate(Vec::new());
        let timeline = result.timeline();

        prop_assert_eq!(timeline.through(), day.through());
        let mut expected_start = minute(0);
        for segment in timeline.segments() {
            prop_assert_eq!(segment.interval().start(), expected_start);
            expected_start = segment.interval().end();
        }
        prop_assert_eq!(expected_start, day.through());
        for pair in timeline.segments().windows(2) {
            let same = pair[0].kind() == pair[1].kind()
                && pair[0].label() == pair[1].label()
                && pair[0].origin() == pair[1].origin();
            prop_assert!(!same, "mergeable neighbours {:?}", pair);
        }
    }

    #[test]
    fn credited_excluded_and_unknown_time_sum_to_the_horizon(day in day()) {
        let result = day.calculate(Vec::new());
        let timeline = result.timeline();

        prop_assert_eq!(
            timeline.credited_work() + timeline.excluded_work() + timeline.unknown_work(),
            day.through().duration_since(minute(0))
        );
        prop_assert_eq!(
            timeline.credited_work(),
            timeline.confirmed_credited_work() + timeline.provisional_work()
        );
        prop_assert_eq!(timeline.credited_coverage().covered_duration(), timeline.credited_work());
        prop_assert_eq!(result.credited_work(), timeline.credited_work());
    }

    #[test]
    fn pending_reviews_cover_only_provisional_time_of_long_locks(day in day()) {
        let result = day.calculate(Vec::new());
        let threshold = SignedDuration::from_mins(day.threshold);
        let provisional: IntervalSet = result
            .timeline()
            .segments()
            .iter()
            .filter(|segment| matches!(segment.kind(), SegmentKind::ProvisionalCredit { .. }))
            .map(|segment| segment.interval())
            .collect();

        for review in result.pending_reviews() {
            prop_assert!(review.locked_episode.duration() >= threshold);
            for covered in &review.provisional_coverage {
                prop_assert!(review.locked_episode.intersect(*covered) == Some(*covered));
                prop_assert_eq!(provisional.intersect(*covered).covered_duration(), covered.duration());
            }
        }
        prop_assert!(result.work_awaiting_review() <= result.timeline().provisional_work());
    }

    #[test]
    fn interval_adjustments_own_exactly_their_spans_and_revoking_them_restores_the_engine(
        day in day(),
        cuts in prop::collection::vec((0i64..WINDOW_MINUTES, any::<bool>()), 0..8),
    ) {
        let automatic = day.calculate(Vec::new());
        let adjustments = interval_adjustments(&day, &cuts);
        let adjusted = day.calculate(adjustments.clone());

        let declared: IntervalSet = adjustments.iter().filter_map(ManualAdjustment::interval).collect();
        let manual: IntervalSet = adjusted
            .timeline()
            .segments()
            .iter()
            .filter(|segment| segment.origin().manual_adjustment_id().is_some())
            .map(|segment| segment.interval())
            .collect();
        prop_assert_eq!(manual, declared);
        prop_assert_eq!(
            adjusted.timeline().credited_work() + adjusted.timeline().excluded_work()
                + adjusted.timeline().unknown_work(),
            day.through().duration_since(minute(0))
        );

        let revocations: Vec<_> = adjustments
            .iter()
            .enumerate()
            .map(|(index, adjustment)| {
                ManualAdjustment::revocation(
                    AdjustmentId::new(format!("revoke-{index}")).unwrap(),
                    workday(),
                    adjustment.id().clone(),
                    adjustment.created_at() + SignedDuration::from_mins(30),
                )
                .unwrap()
            })
            .collect();
        let revoked = day.calculate(adjustments.into_iter().chain(revocations).collect());
        prop_assert_eq!(revoked, automatic);
    }
}
