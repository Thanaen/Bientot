use jiff::civil::date;
use jiff::{SignedDuration, Timestamp};

use super::*;
use crate::accounting::{
    AccountingInput, AccountingPolicy, ActivityCoverage, ActivitySegment, ActivityState,
    AdjustmentId, AutomaticOrigin, PowerState, Recorded, SessionState, TrackingState,
    WorkdayEventHistory, WorkdayEventId, WorkdayStateAnchors, calculate,
};
use crate::adjustments::{ManualAdjustment, ManualClassification};
use crate::intervals::UtcInterval;
use crate::sources::{SourceDiagnosticInterval, SourceHealth};
use crate::time::{LocalWorkday, WorkdayWindow};

fn at(hour: i8, minute: i8, second: i8) -> Timestamp {
    date(2026, 9, 8)
        .at(hour, minute, second, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn start() -> Timestamp {
    at(8, 0, 0)
}

fn interval(start: Timestamp, end: Timestamp) -> UtcInterval {
    UtcInterval::new(start, end).unwrap()
}

fn origin(id: &str) -> AutomaticOrigin {
    AutomaticOrigin::new(id).unwrap()
}

fn workday() -> LocalWorkday {
    LocalWorkday::new(date(2026, 9, 8), "UTC")
}

fn segment(start: Timestamp, end: Timestamp, state: ActivityState) -> ActivitySegment {
    ActivitySegment::new(interval(start, end), state, origin("activitywatch:fixture"))
}

fn calculate_with(
    end: Timestamp,
    history: WorkdayEventHistory,
    adjustments: Vec<ManualAdjustment>,
    segments: Vec<ActivitySegment>,
) -> AccountingResult {
    let input = AccountingInput::new(
        WorkdayWindow {
            workday: workday(),
            start: start(),
            end,
        },
        end,
        AccountingPolicy::new(
            interval(at(12, 0, 0), at(14, 0, 0)),
            SignedDuration::from_mins(90),
        )
        .unwrap(),
        ActivityCoverage::new(interval(start(), end), segments),
        history,
        adjustments,
    )
    .unwrap();
    calculate(&input).unwrap()
}

fn diagnostics(
    end: Timestamp,
    operation: SourceHealth,
    findings: Vec<SourceDiagnosticInterval>,
) -> SourceDiagnostics {
    SourceDiagnostics::new(operation, interval(start(), end), end, findings).unwrap()
}

fn healthy(end: Timestamp, findings: Vec<SourceDiagnosticInterval>) -> SourceDiagnostics {
    diagnostics(end, SourceHealth::healthy("activitywatch", end), findings)
}

fn finding(interval: UtcInterval, kind: SourceIntervalKind) -> SourceDiagnosticInterval {
    SourceDiagnosticInterval::new(interval, kind, "activitywatch").unwrap()
}

#[test]
fn pending_current_tail_is_estimated_and_uncredited() {
    let pending = interval(at(10, 0, 0), at(10, 0, 3));
    let accounting = calculate_with(
        pending.end(),
        WorkdayEventHistory::default(),
        vec![],
        vec![
            segment(start(), pending.start(), ActivityState::Active),
            segment(pending.start(), pending.end(), ActivityState::Unknown),
        ],
    );
    let diagnostics = healthy(
        pending.end(),
        vec![finding(pending, SourceIntervalKind::PendingSettlement)],
    );

    let assessment = assess(&accounting, &diagnostics);

    assert_eq!(assessment.forecast(), ForecastAvailability::Estimated);
    assert_eq!(accounting.timeline().unknown_work(), pending.duration());
    assert_eq!(accounting.credited_work(), SignedDuration::from_hours(2));
}

#[test]
fn seven_minute_startup_gap_with_fresh_current_evidence_is_estimated() {
    let gap = interval(start(), at(8, 7, 0));
    let end = at(10, 0, 0);
    let accounting = calculate_with(
        end,
        WorkdayEventHistory::default(),
        vec![],
        vec![
            segment(gap.start(), gap.end(), ActivityState::Unknown),
            segment(gap.end(), end, ActivityState::Active),
        ],
    );
    let diagnostics = healthy(
        end,
        vec![finding(gap, SourceIntervalKind::UnresolvedCoverage)],
    );

    let assessment = assess(&accounting, &diagnostics);

    assert_eq!(assessment.forecast(), ForecastAvailability::Estimated);
    assert!(
        assessment
            .reasons()
            .contains(&AssessmentReason::HistoricalMissingCoverage)
    );
    assert_eq!(
        accounting.timeline().unknown_work(),
        SignedDuration::from_mins(7)
    );
}

#[test]
fn unresolved_current_frontier_is_unavailable() {
    let end = at(10, 0, 0);
    let gap = interval(at(9, 50, 0), end);
    let accounting = calculate_with(
        end,
        WorkdayEventHistory::default(),
        vec![],
        vec![
            segment(start(), gap.start(), ActivityState::Active),
            segment(gap.start(), gap.end(), ActivityState::Unknown),
        ],
    );

    let assessment = assess(
        &accounting,
        &healthy(
            end,
            vec![finding(gap, SourceIntervalKind::UnresolvedCoverage)],
        ),
    );

    assert_eq!(assessment.forecast(), ForecastAvailability::Unavailable);
    assert!(
        assessment
            .reasons()
            .contains(&AssessmentReason::CurrentMissingCoverage)
    );
}

#[test]
fn final_accounting_override_can_confirm_despite_raw_source_gap() {
    for state in [TrackingState::Paused, TrackingState::Finished] {
        let end = at(10, 0, 0);
        let gap = interval(at(9, 0, 0), end);
        let history = WorkdayEventHistory::new(
            WorkdayStateAnchors {
                tracking: Some(Recorded::new(
                    WorkdayEventId(1),
                    start() - SignedDuration::from_mins(1),
                    state,
                    origin("worktime:tracking"),
                )),
                ..WorkdayStateAnchors::default()
            },
            [],
        );
        let accounting = calculate_with(
            end,
            history,
            vec![],
            vec![
                segment(start(), gap.start(), ActivityState::Active),
                segment(gap.start(), gap.end(), ActivityState::Unknown),
            ],
        );
        let operation = SourceHealth::degraded(
            "activitywatch",
            SourceFreshness::Stale,
            SourceQuality::Complete,
            "watcher-stale",
            Some(at(9, 0, 0)),
        )
        .unwrap();
        let diagnostics = diagnostics(
            end,
            operation,
            vec![finding(gap, SourceIntervalKind::UnresolvedCoverage)],
        );

        let assessment = assess(&accounting, &diagnostics);

        assert_eq!(
            assessment.forecast(),
            ForecastAvailability::Confirmed,
            "{state:?}"
        );
        assert_eq!(accounting.timeline().unknown_work(), SignedDuration::ZERO);
        assert!(
            assessment
                .reasons()
                .contains(&AssessmentReason::SourceStale)
        );
    }
}

#[test]
fn suspended_power_state_confirms_unknown_source_coverage_as_excluded() {
    let end = at(10, 0, 0);
    let gap = interval(at(9, 0, 0), end);
    let history = WorkdayEventHistory::new(
        WorkdayStateAnchors {
            power: Some(Recorded::new(
                WorkdayEventId(1),
                start() - SignedDuration::from_mins(1),
                PowerState::Suspended,
                origin("worktime:power"),
            )),
            ..WorkdayStateAnchors::default()
        },
        [],
    );
    let accounting = calculate_with(
        end,
        history,
        vec![],
        vec![
            segment(start(), gap.start(), ActivityState::Active),
            segment(gap.start(), gap.end(), ActivityState::Unknown),
        ],
    );
    let diagnostics = healthy(
        end,
        vec![finding(gap, SourceIntervalKind::UnresolvedCoverage)],
    );

    let assessment = assess(&accounting, &diagnostics);

    assert_eq!(assessment.forecast(), ForecastAvailability::Confirmed);
    assert_eq!(accounting.timeline().unknown_work(), SignedDuration::ZERO);
}

#[test]
fn manual_reclassification_confirms_unknown_source_coverage_after_accounting() {
    let end = at(10, 0, 0);
    let gap = interval(at(9, 0, 0), end);
    let adjustment = ManualAdjustment::reclassification(
        AdjustmentId::new("resolve-source-gap").unwrap(),
        workday(),
        gap,
        ManualClassification::Excluded,
        end,
    )
    .unwrap();
    let accounting = calculate_with(
        end,
        WorkdayEventHistory::default(),
        vec![adjustment],
        vec![
            segment(start(), gap.start(), ActivityState::Active),
            segment(gap.start(), gap.end(), ActivityState::Unknown),
        ],
    );
    let diagnostics = healthy(
        end,
        vec![finding(gap, SourceIntervalKind::UnresolvedCoverage)],
    );

    let assessment = assess(&accounting, &diagnostics);

    assert_eq!(assessment.forecast(), ForecastAvailability::Confirmed);
    assert_eq!(accounting.timeline().unknown_work(), SignedDuration::ZERO);
    assert_eq!(diagnostics.intervals().len(), 1);
}

#[test]
fn open_long_lock_review_remains_a_confirmed_provisional_forecast() {
    let end = at(10, 0, 0);
    let history = WorkdayEventHistory::new(
        WorkdayStateAnchors::default(),
        [Recorded::new(
            WorkdayEventId(1),
            at(8, 0, 0),
            SessionState::Locked,
            origin("worktime:session"),
        )
        .into()],
    );
    let input = AccountingInput::new(
        WorkdayWindow {
            workday: workday(),
            start: start(),
            end: at(18, 0, 0),
        },
        end,
        AccountingPolicy::new(
            interval(at(12, 0, 0), at(14, 0, 0)),
            SignedDuration::from_mins(90),
        )
        .unwrap(),
        ActivityCoverage::new(
            interval(start(), end),
            [segment(start(), end, ActivityState::Active)],
        ),
        history,
        vec![],
    )
    .unwrap();
    let accounting = calculate(&input).unwrap();

    let assessment = assess(&accounting, &healthy(end, vec![]));

    assert_eq!(assessment.forecast(), ForecastAvailability::Confirmed);
    assert!(
        assessment
            .reasons()
            .contains(&AssessmentReason::ProvisionalLockReview)
    );
    assert!(
        accounting
            .timeline()
            .segments()
            .iter()
            .any(|segment| matches!(segment.kind(), SegmentKind::ProvisionalCredit { .. }))
    );
    assert!(
        accounting
            .pending_reviews()
            .iter()
            .any(|review| review.is_open)
    );
}

#[test]
fn reasons_are_distinct_and_sorted() {
    let assessment = AccountingAssessment::new(
        ForecastAvailability::Estimated,
        [
            AssessmentReason::SourceStale,
            AssessmentReason::PendingSettlement,
            AssessmentReason::SourceStale,
        ],
    );

    assert_eq!(
        assessment.reasons(),
        [
            AssessmentReason::PendingSettlement,
            AssessmentReason::SourceStale
        ]
    );
}
