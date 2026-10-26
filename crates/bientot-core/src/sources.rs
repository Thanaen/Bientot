//! What the services read activity through, ported from
//! `Abstractions/Sources/IActivitySourcePort.cs`, and how a read becomes accounting evidence,
//! ported from `Services/ActivityEvidenceNormalizer.cs`.
//!
//! ActivityWatch is not ported in v1, so a read has no bucket to choose and no backoff to bypass:
//! the .NET `SelectionRequired` result, `ActivityBucketId` and `BypassBackoff` are left out.

use bientot_domain::accounting::{
    ActivityCoverage, ActivityState, AutomaticOrigin, COVERAGE_GAP_ORIGIN,
};
use bientot_domain::intervals::{IntervalSet, UtcInterval};
use bientot_domain::sources::{
    SourceDiagnosticInterval, SourceDiagnostics, SourceFreshness, SourceHealth, SourceIntervalKind,
    SourceQuality, complete_unknown_gaps,
};
use bientot_store::activity::ActivityError;
use jiff::{SignedDuration, Timestamp};

/// The origin of the unknown time the normalizer adds when the source cannot answer. It keeps
/// the .NET identifier whatever the source, as [`COVERAGE_GAP_ORIGIN`] does.
const UNAVAILABLE_ORIGIN: &str = "activitywatch:unavailable";
/// The longest gap between two known segments that is a rounding seam of the source rather than
/// missing evidence.
const INTERIOR_COMPATIBILITY_SEAM_LIMIT: SignedDuration = SignedDuration::from_millis(2);

/// A source of activity evidence: the built-in source, or a demo.
pub trait ActivitySource {
    /// The activity the source has settled in `interval`, and how the source is doing.
    fn read(&self, interval: UtcInterval) -> Result<ActivityReadResult, ActivityError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivityReadResult {
    /// What the source knows; the time it leaves uncovered is classified by [`normalize`].
    Snapshot {
        coverage: ActivityCoverage,
        health: SourceHealth,
    },
    /// The source cannot answer, so the whole read is unknown.
    Unavailable { health: SourceHealth },
}

impl ActivityReadResult {
    pub fn health(&self) -> &SourceHealth {
        match self {
            Self::Snapshot { health, .. } | Self::Unavailable { health } => health,
        }
    }
}

/// The coverage the accounting engine reads, every instant of the bounds covered, and what the
/// source could not settle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedActivityEvidence {
    pub coverage: ActivityCoverage,
    pub diagnostics: SourceDiagnostics,
}

/// Fills what `result` leaves uncovered in `bounds` with unknown time and classifies each unknown
/// interval: a live tail younger than `pending_horizon` may still settle, a seam of at most 2 ms
/// between known evidence is the source's rounding, anything else is unresolved.
pub fn normalize(
    bounds: UtcInterval,
    observed_at: Timestamp,
    result: &ActivityReadResult,
    pending_horizon: SignedDuration,
) -> NormalizedActivityEvidence {
    match result {
        ActivityReadResult::Snapshot { coverage, health } => {
            normalize_snapshot(bounds, observed_at, coverage, health, pending_horizon)
        }
        ActivityReadResult::Unavailable { health } => NormalizedActivityEvidence {
            coverage: ActivityCoverage::unknown(bounds, origin(UNAVAILABLE_ORIGIN)),
            diagnostics: diagnostics(
                health,
                bounds,
                observed_at,
                [(bounds, SourceIntervalKind::UnresolvedCoverage)],
            ),
        },
    }
}

fn normalize_snapshot(
    bounds: UtcInterval,
    observed_at: Timestamp,
    snapshot: &ActivityCoverage,
    health: &SourceHealth,
    pending_horizon: SignedDuration,
) -> NormalizedActivityEvidence {
    let gap_origin = origin(COVERAGE_GAP_ORIGIN);
    let coverage = complete_unknown_gaps(bounds, snapshot, &gap_origin);
    let missing: Vec<UtcInterval> = coverage
        .segments()
        .iter()
        .filter(|segment| segment.state == ActivityState::Unknown && segment.origin == gap_origin)
        .map(|segment| segment.interval)
        .collect();
    // Clipped to the bounds, where .NET would fail on a retained snapshot wider than the read.
    let explicit_unknown: Vec<UtcInterval> = snapshot
        .segments()
        .iter()
        .filter(|segment| segment.state == ActivityState::Unknown)
        .filter_map(|segment| segment.interval.intersect(bounds))
        .collect();
    let complete_current =
        health.freshness == SourceFreshness::Fresh && health.quality == SourceQuality::Complete;
    let components = IntervalSet::new(missing.iter().chain(&explicit_unknown).copied());
    let classified = components.iter().map(|&component| {
        let whole_missing = missing.contains(&component)
            && !explicit_unknown
                .iter()
                .any(|interval| interval.touches_or_overlaps(component));
        let kind = if whole_missing && complete_current {
            classify_missing(component, bounds, observed_at, snapshot, pending_horizon)
        } else {
            SourceIntervalKind::UnresolvedCoverage
        };
        (component, kind)
    });
    NormalizedActivityEvidence {
        diagnostics: diagnostics(health, bounds, observed_at, classified),
        coverage,
    }
}

/// A gap with nothing unknown reported in it, from a fresh and complete read.
fn classify_missing(
    component: UtcInterval,
    bounds: UtcInterval,
    observed_at: Timestamp,
    snapshot: &ActivityCoverage,
    pending_horizon: SignedDuration,
) -> SourceIntervalKind {
    let has_known_predecessor = has_known_boundary(snapshot, component.start(), Side::Before);
    let settlement_age = observed_at.duration_since(component.start());
    if component.end() == bounds.end()
        && component.duration() <= pending_horizon
        && (SignedDuration::ZERO..=pending_horizon).contains(&settlement_age)
        && has_known_predecessor
    {
        return SourceIntervalKind::PendingSettlement;
    }
    let has_known_successor = has_known_boundary(snapshot, component.end(), Side::After);
    if component.start() > bounds.start()
        && component.end() < bounds.end()
        && component.duration() <= INTERIOR_COMPATIBILITY_SEAM_LIMIT
        && has_known_predecessor
        && has_known_successor
    {
        SourceIntervalKind::InteriorCompatibilitySeam
    } else {
        SourceIntervalKind::UnresolvedCoverage
    }
}

#[derive(Clone, Copy)]
enum Side {
    Before,
    After,
}

/// Whether known evidence ends at `boundary` (before it) or starts there (after it).
fn has_known_boundary(coverage: &ActivityCoverage, boundary: Timestamp, side: Side) -> bool {
    coverage.segments().iter().any(|segment| {
        segment.state != ActivityState::Unknown
            && match side {
                Side::Before => segment.interval.end() == boundary,
                Side::After => segment.interval.start() == boundary,
            }
    })
}

fn origin(source_id: &str) -> AutomaticOrigin {
    AutomaticOrigin::new(source_id).expect("the normalizer's origins are not blank")
}

/// # Panics
///
/// When `health` names a blank source, which no source does.
fn diagnostics(
    health: &SourceHealth,
    bounds: UtcInterval,
    observed_at: Timestamp,
    intervals: impl IntoIterator<Item = (UtcInterval, SourceIntervalKind)>,
) -> SourceDiagnostics {
    let intervals = intervals.into_iter().map(|(interval, kind)| {
        SourceDiagnosticInterval::new(interval, kind, &health.source_id)
            .expect("a source names itself")
    });
    SourceDiagnostics::new(health.clone(), bounds, observed_at, intervals)
        .expect("the unknown components are disjoint and inside the bounds")
}

#[cfg(test)]
mod tests;
