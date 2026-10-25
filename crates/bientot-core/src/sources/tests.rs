//! Ported from `ActivityEvidenceNormalizerTests.cs`.

use bientot_domain::accounting::{ActivitySegment, ActivityState};
use bientot_domain::configuration::ActivityWatchSettlementContract;
use bientot_domain::sources::SourceIntervalKind::{
    InteriorCompatibilitySeam, PendingSettlement, UnresolvedCoverage,
};
use jiff::{SignedDuration, Timestamp};

use super::*;
use crate::test_support::utc;

/// One .NET tick, the precision of the .NET instants these tests were written against.
const TICK: SignedDuration = SignedDuration::from_nanos(100);

fn start() -> Timestamp {
    utc(2026, 9, 8, 8)
}

fn bounds(duration: SignedDuration) -> UtcInterval {
    UtcInterval::new(start(), start() + duration).unwrap()
}

fn interval(start: Timestamp, end: Timestamp) -> UtcInterval {
    UtcInterval::new(start, end).unwrap()
}

fn fixture_origin() -> AutomaticOrigin {
    AutomaticOrigin::new("activitywatch:fixture").unwrap()
}

fn known(start: Timestamp, end: Timestamp) -> ActivitySegment {
    ActivitySegment::new(
        interval(start, end),
        ActivityState::Active,
        fixture_origin(),
    )
}

fn snapshot(
    bounds: UtcInterval,
    segments: impl IntoIterator<Item = ActivitySegment>,
) -> ActivityReadResult {
    snapshot_with(
        bounds,
        segments,
        SourceHealth::healthy("activitywatch", bounds.end()),
    )
}

fn snapshot_with(
    bounds: UtcInterval,
    segments: impl IntoIterator<Item = ActivitySegment>,
    health: SourceHealth,
) -> ActivityReadResult {
    ActivityReadResult::Snapshot {
        coverage: ActivityCoverage::new(bounds, segments),
        health,
    }
}

fn default_horizon() -> SignedDuration {
    ActivityWatchSettlementContract::default().horizon()
}

/// Normalized as observed at the end of the read.
fn normalize_at_end(
    bounds: UtcInterval,
    result: &ActivityReadResult,
    pending_horizon: SignedDuration,
) -> NormalizedActivityEvidence {
    normalize(bounds, bounds.end(), result, pending_horizon)
}

fn findings(normalized: &NormalizedActivityEvidence) -> Vec<(UtcInterval, SourceIntervalKind)> {
    normalized
        .diagnostics
        .intervals()
        .iter()
        .map(|finding| (finding.interval(), finding.kind()))
        .collect()
}

fn total(coverage: &ActivityCoverage, state: ActivityState) -> SignedDuration {
    coverage
        .segments()
        .iter()
        .filter(|segment| segment.state == state)
        .fold(SignedDuration::ZERO, |total, segment| {
            total + segment.interval.duration()
        })
}

#[test]
fn fresh_complete_trailing_gap_after_known_evidence_is_pending_and_uncredited() {
    let bounds = bounds(SignedDuration::from_hours(4));
    let evidence_end = bounds.end() - SignedDuration::from_millis(2_500);

    let normalized = normalize_at_end(
        bounds,
        &snapshot(bounds, [known(bounds.start(), evidence_end)]),
        default_horizon(),
    );

    assert_eq!(
        findings(&normalized),
        [(interval(evidence_end, bounds.end()), PendingSettlement)]
    );
    assert_eq!(
        total(&normalized.coverage, ActivityState::Active),
        SignedDuration::from_hours(4) - SignedDuration::from_millis(2_500)
    );
    assert_eq!(
        total(&normalized.coverage, ActivityState::Unknown),
        SignedDuration::from_millis(2_500)
    );
}

#[test]
fn empty_short_response_is_unresolved_instead_of_pending() {
    let bounds = bounds(SignedDuration::from_millis(2_500));

    let normalized = normalize_at_end(bounds, &snapshot(bounds, []), default_horizon());

    assert_eq!(findings(&normalized), [(bounds, UnresolvedCoverage)]);
    assert_eq!(
        total(&normalized.coverage, ActivityState::Active),
        SignedDuration::ZERO
    );
}

#[test]
fn settlement_horizon_uses_timeout_poll_and_jitter_without_minute_rounding() {
    let horizon = ActivityWatchSettlementContract {
        afk_timeout: SignedDuration::from_secs(10),
        poll_interval: SignedDuration::from_secs(2),
        maximum_jitter: SignedDuration::from_secs(1),
    }
    .horizon();
    let bounds = bounds(SignedDuration::from_mins(30));
    let inside_end = bounds.end() - horizon;
    let outside_end = inside_end - TICK;

    let inside = normalize_at_end(
        bounds,
        &snapshot(bounds, [known(bounds.start(), inside_end)]),
        horizon,
    );
    let outside = normalize_at_end(
        bounds,
        &snapshot(bounds, [known(bounds.start(), outside_end)]),
        horizon,
    );

    assert_eq!(
        findings(&inside),
        [(interval(inside_end, bounds.end()), PendingSettlement)]
    );
    assert_eq!(
        findings(&outside),
        [(interval(outside_end, bounds.end()), UnresolvedCoverage)]
    );
}

#[test]
fn historical_trailing_gap_uses_its_age_at_real_observation() {
    let horizon = default_horizon();
    let bounds = bounds(SignedDuration::from_hours(24));
    let gap_start = bounds.end() - SignedDuration::from_millis(2_500);
    let snapshot = snapshot(bounds, [known(bounds.start(), gap_start)]);

    for (ticks_past_horizon, expected) in [(0, PendingSettlement), (1, UnresolvedCoverage)] {
        let observed_at = gap_start + horizon + TICK * ticks_past_horizon;

        let normalized = normalize(bounds, observed_at, &snapshot, horizon);

        assert_eq!(
            findings(&normalized),
            [(interval(gap_start, bounds.end()), expected)],
            "{ticks_past_horizon} ticks past the horizon"
        );
        assert_eq!(normalized.diagnostics.observed_at(), observed_at);
    }
}

#[test]
fn healthy_http_response_does_not_hide_a_stopped_watcher_tail() {
    let horizon = default_horizon();
    let bounds = bounds(SignedDuration::from_mins(30));
    let evidence_end = bounds.end() - horizon - SignedDuration::from_secs(1);

    let normalized = normalize_at_end(
        bounds,
        &snapshot(bounds, [known(bounds.start(), evidence_end)]),
        horizon,
    );

    assert_eq!(
        normalized.diagnostics.operation().freshness,
        SourceFreshness::Fresh
    );
    assert_eq!(
        findings(&normalized),
        [(interval(evidence_end, bounds.end()), UnresolvedCoverage)]
    );
}

#[test]
fn explicit_unknown_and_stale_trailing_gaps_are_never_pending() {
    let bounds = bounds(SignedDuration::from_mins(30));
    let tail_start = bounds.end() - SignedDuration::from_secs(2);
    let tail = interval(tail_start, bounds.end());
    let explicit_unknown = normalize_at_end(
        bounds,
        &snapshot(
            bounds,
            [
                known(bounds.start(), tail_start),
                ActivitySegment::new(tail, ActivityState::Unknown, fixture_origin()),
            ],
        ),
        default_horizon(),
    );
    let stale = normalize_at_end(
        bounds,
        &snapshot_with(
            bounds,
            [known(bounds.start(), tail_start)],
            SourceHealth::degraded(
                "activitywatch",
                SourceFreshness::Stale,
                SourceQuality::Complete,
                "offline",
                Some(bounds.end()),
            )
            .unwrap(),
        ),
        default_horizon(),
    );

    assert_eq!(findings(&explicit_unknown), [(tail, UnresolvedCoverage)]);
    assert_eq!(findings(&stale), [(tail, UnresolvedCoverage)]);
}

#[test]
fn only_strict_interior_missing_seams_at_the_named_limit_are_compatible() {
    let bounds = bounds(SignedDuration::from_mins(30));
    let seam_start = bounds.start() + SignedDuration::from_mins(10);

    for (milliseconds, expected) in [(2, InteriorCompatibilitySeam), (3, UnresolvedCoverage)] {
        let seam_end = seam_start + SignedDuration::from_millis(milliseconds);

        let normalized = normalize_at_end(
            bounds,
            &snapshot(
                bounds,
                [
                    known(bounds.start(), seam_start),
                    known(seam_end, bounds.end()),
                ],
            ),
            default_horizon(),
        );

        assert_eq!(
            findings(&normalized),
            [(interval(seam_start, seam_end), expected)],
            "{milliseconds} ms seam"
        );
        assert_eq!(
            total(&normalized.coverage, ActivityState::Unknown),
            SignedDuration::from_millis(milliseconds)
        );
    }
}

#[test]
fn two_millisecond_query_edge_gap_is_not_an_interior_compatibility_seam() {
    let bounds = bounds(SignedDuration::from_mins(30));
    let evidence_start = bounds.start() + SignedDuration::from_millis(2);

    let normalized = normalize_at_end(
        bounds,
        &snapshot(bounds, [known(evidence_start, bounds.end())]),
        default_horizon(),
    );

    assert_eq!(
        findings(&normalized),
        [(interval(bounds.start(), evidence_start), UnresolvedCoverage)]
    );
}
