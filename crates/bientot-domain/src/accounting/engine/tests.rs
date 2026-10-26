mod long_lock_review;
mod overnight_lock;
mod rebuilt_session;
mod recurring_appointment;

use jiff::civil::date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use crate::accounting::{
    AccountedOrigin, AccountedSegment, AccountingInput, AccountingLabel, AccountingPolicy,
    AccountingResult, ActivityCoverage, ActivitySegment, ActivityState, AutomaticOrigin,
    PowerState, Recorded, RecordedTransition, SegmentKind, SessionState, TrackingState,
    WorkdayEventHistory, WorkdayEventId, WorkdayStateAnchors, calculate,
};
use crate::intervals::UtcInterval;
use crate::time::{LocalWorkday, WorkdayWindow};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Type {
    Credited,
    Provisional,
    Excluded,
    Unknown,
}

/// The segment's variant, interval and label, as the .NET tests compare them.
pub(super) fn shape(segment: &AccountedSegment) -> (Type, UtcInterval, AccountingLabel) {
    let kind = match segment.kind() {
        SegmentKind::Credited => Type::Credited,
        SegmentKind::ProvisionalCredit { .. } => Type::Provisional,
        SegmentKind::Excluded => Type::Excluded,
        SegmentKind::Unknown => Type::Unknown,
    };
    (kind, segment.interval(), segment.label())
}

pub(super) fn shapes(result: &AccountingResult) -> Vec<(Type, UtcInterval, AccountingLabel)> {
    result.timeline().segments().iter().map(shape).collect()
}

pub(super) fn single(result: &AccountingResult) -> &AccountedSegment {
    match result.timeline().segments() {
        [segment] => segment,
        segments => panic!("expected one segment, got {segments:?}"),
    }
}

pub(super) fn single_labelled(
    result: &AccountingResult,
    label: AccountingLabel,
) -> &AccountedSegment {
    let matching: Vec<_> = result
        .timeline()
        .segments()
        .iter()
        .filter(|segment| segment.label() == label)
        .collect();
    match matching.as_slice() {
        [segment] => segment,
        segments => panic!("expected one {label:?} segment, got {segments:?}"),
    }
}

pub(super) fn manual_id(segment: &AccountedSegment) -> &str {
    segment
        .origin()
        .manual_adjustment_id()
        .expect("a manual origin")
        .as_str()
}

pub(super) fn origin(id: &str) -> AutomaticOrigin {
    AutomaticOrigin::new(id).unwrap()
}

pub(super) fn minutes(minutes: i64) -> SignedDuration {
    SignedDuration::from_mins(minutes)
}

pub(super) fn hours(hours: i64) -> SignedDuration {
    SignedDuration::from_hours(hours)
}

fn at(hour: i8, minute: i8) -> Timestamp {
    date(2026, 8, 3)
        .at(hour, minute, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn interval(start: Timestamp, end: Timestamp) -> UtcInterval {
    UtcInterval::new(start, end).unwrap()
}

fn activity(start: Timestamp, end: Timestamp, state: ActivityState) -> ActivitySegment {
    ActivitySegment::new(interval(start, end), state, origin("activitywatch:afk"))
}

fn session(id: i64, occurred_at: Timestamp, state: SessionState) -> Recorded<SessionState> {
    Recorded::new(
        WorkdayEventId(id),
        occurred_at,
        state,
        origin("windows:session"),
    )
}

fn power(id: i64, occurred_at: Timestamp, state: PowerState) -> Recorded<PowerState> {
    Recorded::new(
        WorkdayEventId(id),
        occurred_at,
        state,
        origin("windows:power"),
    )
}

fn tracking(id: i64, occurred_at: Timestamp, state: TrackingState) -> Recorded<TrackingState> {
    Recorded::new(
        WorkdayEventId(id),
        occurred_at,
        state,
        origin("worktime:tracking"),
    )
}

fn anchored(anchors: WorkdayStateAnchors) -> WorkdayEventHistory {
    WorkdayEventHistory::new(anchors, [])
}

fn calculate_between(
    start: Timestamp,
    end: Timestamp,
    activity: Vec<ActivitySegment>,
    history: WorkdayEventHistory,
) -> AccountingResult {
    let window = WorkdayWindow {
        workday: LocalWorkday::new(date(2026, 8, 3), "UTC"),
        start,
        end,
    };
    let policy = AccountingPolicy::new(interval(at(12, 0), at(14, 0)), minutes(90)).unwrap();
    let input = AccountingInput::new(
        window,
        end,
        policy,
        ActivityCoverage::new(interval(start, end), activity),
        history,
        Vec::new(),
    )
    .unwrap();
    calculate(&input).unwrap()
}

#[test]
fn overlapping_active_evidence_produces_one_credited_timeline() {
    let (start, end) = (at(8, 0), at(12, 0));

    let result = calculate_between(
        start,
        end,
        vec![
            activity(start, at(11, 0), ActivityState::Active),
            activity(at(9, 0), end, ActivityState::Active),
        ],
        WorkdayEventHistory::default(),
    );

    assert_eq!(
        shapes(&result),
        [(
            Type::Credited,
            interval(start, end),
            AccountingLabel::ActiveWork
        )]
    );
    assert_eq!(result.credited_work(), hours(4));
}

#[test]
fn lock_crossing_lunch_splits_into_provisional_and_excluded_segments() {
    let (start, end) = (at(11, 0), at(15, 0));
    let history = WorkdayEventHistory::new(
        WorkdayStateAnchors::default(),
        [
            session(1, start, SessionState::Locked).into(),
            session(2, end, SessionState::Unlocked).into(),
        ],
    );

    let result = calculate_between(
        start,
        end,
        vec![activity(start, end, ActivityState::Active)],
        history,
    );

    assert_eq!(
        shapes(&result),
        [
            (
                Type::Provisional,
                interval(start, at(12, 0)),
                AccountingLabel::LockedWork
            ),
            (
                Type::Excluded,
                interval(at(12, 0), at(14, 0)),
                AccountingLabel::LunchLock
            ),
            (
                Type::Provisional,
                interval(at(14, 0), end),
                AccountingLabel::LockedWork
            ),
        ]
    );
    let segments = result.timeline().segments();
    assert_eq!(segments[0].kind(), segments[2].kind());
    assert_eq!(result.credited_work(), hours(2));
    assert_eq!(result.pending_reviews().len(), 1);
}

#[test]
fn afk_evidence_is_excluded() {
    let (start, end) = (at(8, 0), at(10, 0));

    let result = calculate_between(
        start,
        end,
        vec![activity(start, end, ActivityState::Afk)],
        WorkdayEventHistory::default(),
    );

    let segment = single(&result);
    assert_eq!(segment.kind(), &SegmentKind::Excluded);
    assert_eq!(segment.label(), AccountingLabel::Afk);
    assert_eq!(
        segment.origin(),
        &AccountedOrigin::Automatic(origin("activitywatch:afk"))
    );
    assert_eq!(result.credited_work(), SignedDuration::ZERO);
}

#[test]
fn suspend_wins_over_lock_and_active_evidence() {
    let (start, end) = (at(8, 0), at(12, 0));
    let history = WorkdayEventHistory::new(
        WorkdayStateAnchors {
            session: Some(session(1, at(7, 30), SessionState::Unlocked)),
            power: Some(power(2, at(7, 30), PowerState::Active)),
            tracking: None,
        },
        [
            session(3, at(9, 0), SessionState::Locked).into(),
            power(4, at(9, 30), PowerState::Suspended).into(),
            power(5, at(10, 30), PowerState::Active).into(),
            session(6, at(11, 0), SessionState::Unlocked).into(),
        ],
    );

    let result = calculate_between(
        start,
        end,
        vec![activity(start, end, ActivityState::Active)],
        history,
    );

    let suspended = single_labelled(&result, AccountingLabel::Suspended);
    assert_eq!(
        shape(suspended),
        (
            Type::Excluded,
            interval(at(9, 30), at(10, 30)),
            AccountingLabel::Suspended
        )
    );
    assert_eq!(
        suspended.origin(),
        &AccountedOrigin::Automatic(origin("windows:power"))
    );
    assert_eq!(result.credited_work(), hours(3));
}

#[test]
fn outside_lunch_lock_is_provisional_without_double_counting_activity() {
    let (start, end) = (at(8, 0), at(11, 0));
    let history = WorkdayEventHistory::new(
        WorkdayStateAnchors {
            session: Some(session(1, at(7, 30), SessionState::Unlocked)),
            ..WorkdayStateAnchors::default()
        },
        [
            session(2, at(9, 0), SessionState::Locked).into(),
            session(3, at(10, 0), SessionState::Unlocked).into(),
        ],
    );

    let result = calculate_between(
        start,
        end,
        vec![activity(start, end, ActivityState::Active)],
        history,
    );

    assert_eq!(
        shapes(&result),
        [
            (
                Type::Credited,
                interval(start, at(9, 0)),
                AccountingLabel::ActiveWork
            ),
            (
                Type::Provisional,
                interval(at(9, 0), at(10, 0)),
                AccountingLabel::LockedWork
            ),
            (
                Type::Credited,
                interval(at(10, 0), end),
                AccountingLabel::ActiveWork
            ),
        ]
    );
    assert_eq!(result.credited_work(), hours(3));
    assert!(result.pending_reviews().is_empty());
}

#[test]
fn explicit_tracking_state_excludes_automatic_credit() {
    for (tracking_state, expected_label) in [
        (TrackingState::Paused, AccountingLabel::ManualPause),
        (TrackingState::Finished, AccountingLabel::DayFinished),
    ] {
        let (start, end) = (at(8, 0), at(9, 0));
        let history = anchored(WorkdayStateAnchors {
            tracking: Some(tracking(1, at(7, 30), tracking_state)),
            ..WorkdayStateAnchors::default()
        });

        let result = calculate_between(
            start,
            end,
            vec![activity(start, end, ActivityState::Active)],
            history,
        );

        let segment = single(&result);
        assert_eq!(segment.kind(), &SegmentKind::Excluded, "{tracking_state:?}");
        assert_eq!(segment.label(), expected_label);
        assert_eq!(
            segment.origin(),
            &AccountedOrigin::Automatic(origin("worktime:tracking"))
        );
        assert_eq!(result.credited_work(), SignedDuration::ZERO);
    }
}

#[test]
fn separate_pre_window_anchors_reconstruct_each_state_lane() {
    let (start, end) = (at(8, 0), at(9, 0));
    let evidence = || vec![activity(start, end, ActivityState::Active)];

    let locked = calculate_between(
        start,
        end,
        evidence(),
        anchored(WorkdayStateAnchors {
            session: Some(session(1, at(7, 30), SessionState::Locked)),
            ..WorkdayStateAnchors::default()
        }),
    );
    let suspended = calculate_between(
        start,
        end,
        evidence(),
        anchored(WorkdayStateAnchors {
            power: Some(power(2, at(7, 30), PowerState::Suspended)),
            ..WorkdayStateAnchors::default()
        }),
    );
    let paused = calculate_between(
        start,
        end,
        evidence(),
        anchored(WorkdayStateAnchors {
            tracking: Some(tracking(3, at(7, 30), TrackingState::Paused)),
            ..WorkdayStateAnchors::default()
        }),
    );

    assert_eq!(single(&locked).label(), AccountingLabel::OvernightLock);
    assert_eq!(single(&suspended).label(), AccountingLabel::Suspended);
    assert_eq!(single(&paused).label(), AccountingLabel::ManualPause);
}

#[test]
fn transition_at_window_start_overrides_the_pre_window_anchor() {
    let (start, end) = (at(8, 0), at(9, 0));
    let history = WorkdayEventHistory::new(
        WorkdayStateAnchors {
            power: Some(power(1, at(7, 30), PowerState::Suspended)),
            ..WorkdayStateAnchors::default()
        },
        [power(2, start, PowerState::Active).into()],
    );

    let result = calculate_between(
        start,
        end,
        vec![activity(start, end, ActivityState::Active)],
        history,
    );

    assert_eq!(single(&result).kind(), &SegmentKind::Credited);
}

#[test]
fn explicit_unknown_lifecycle_state_defers_to_independent_activity_evidence() {
    for (session_lane, activity_state, expected_label) in [
        (true, ActivityState::Active, AccountingLabel::ActiveWork),
        (false, ActivityState::Active, AccountingLabel::ActiveWork),
        (true, ActivityState::Afk, AccountingLabel::Afk),
        (false, ActivityState::Afk, AccountingLabel::Afk),
    ] {
        let (start, end) = (at(8, 0), at(9, 0));
        let transition: RecordedTransition = if session_lane {
            session(1, start, SessionState::Unknown).into()
        } else {
            power(1, start, PowerState::Unknown).into()
        };

        let result = calculate_between(
            start,
            end,
            vec![activity(start, end, activity_state)],
            WorkdayEventHistory::new(WorkdayStateAnchors::default(), [transition]),
        );

        assert_eq!(
            single(&result).label(),
            expected_label,
            "session lane: {session_lane}, {activity_state:?}"
        );
    }
}

#[test]
fn missing_and_malformed_evidence_fail_closed_as_unknown() {
    let (start, end) = (at(8, 0), at(9, 0));

    let missing = calculate_between(start, end, vec![], WorkdayEventHistory::default());
    let malformed = calculate_between(
        start,
        end,
        vec![ActivitySegment::new(
            interval(start, end),
            ActivityState::Unknown,
            origin("activitywatch:malformed"),
        )],
        WorkdayEventHistory::default(),
    );

    assert_eq!(single(&missing).kind(), &SegmentKind::Unknown);
    assert_eq!(single(&malformed).kind(), &SegmentKind::Unknown);
    assert_eq!(
        single(&missing).origin(),
        &AccountedOrigin::Automatic(origin("activity:missing"))
    );
    assert_eq!(
        single(&malformed).origin(),
        &AccountedOrigin::Automatic(origin("activitywatch:malformed"))
    );
    assert_eq!(missing.credited_work(), SignedDuration::ZERO);
    assert_eq!(malformed.credited_work(), SignedDuration::ZERO);
}

#[test]
fn conflicting_activity_overlap_fails_closed_only_for_the_conflict() {
    let (start, end) = (at(8, 0), at(11, 0));

    let result = calculate_between(
        start,
        end,
        vec![
            activity(start, at(10, 0), ActivityState::Active),
            activity(at(9, 0), end, ActivityState::Afk),
        ],
        WorkdayEventHistory::default(),
    );

    assert_eq!(
        shapes(&result),
        [
            (
                Type::Credited,
                interval(start, at(9, 0)),
                AccountingLabel::ActiveWork
            ),
            (
                Type::Unknown,
                interval(at(9, 0), at(10, 0)),
                AccountingLabel::Unknown
            ),
            (
                Type::Excluded,
                interval(at(10, 0), end),
                AccountingLabel::Afk
            ),
        ]
    );
    assert_eq!(result.credited_work(), hours(1));
}

/// The domain half of `TrackingControlTests.FinishExcludesTheRestOfTheCurrentWorkday`: the
/// .NET test records the finish through the app's tracking service, here it is recorded directly.
#[test]
fn finish_excludes_the_rest_of_the_current_workday() {
    let monday = LocalWorkday::new(date(2026, 8, 24), "UTC");
    let at_monday = |hour| {
        date(2026, 8, 24)
            .at(hour, 0, 0, 0)
            .to_zoned(TimeZone::UTC)
            .unwrap()
            .timestamp()
    };
    let window = WorkdayWindow {
        workday: monday,
        start: at_monday(0),
        end: "2026-08-25T00:00:00Z".parse().unwrap(),
    };
    let history = WorkdayEventHistory::new(
        WorkdayStateAnchors::default(),
        [Recorded::new(
            WorkdayEventId(1),
            at_monday(10),
            TrackingState::Finished,
            origin("tracking-control-test"),
        )
        .into()],
    );
    let observed = interval(window.start, at_monday(17));
    let input = AccountingInput::new(
        window,
        at_monday(17),
        AccountingPolicy::new(interval(at_monday(12), at_monday(14)), minutes(90)).unwrap(),
        ActivityCoverage::new(
            observed,
            [ActivitySegment::new(
                observed,
                ActivityState::Active,
                origin("activity"),
            )],
        ),
        history,
        Vec::new(),
    )
    .unwrap();

    let result = calculate(&input).unwrap();

    assert_eq!(
        single_labelled(&result, AccountingLabel::DayFinished).interval(),
        interval(at_monday(10), at_monday(17))
    );
}
