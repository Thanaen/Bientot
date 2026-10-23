use crate::accounting::{ActivityCoverage, ActivitySegment, ActivityState, AutomaticOrigin};
use crate::intervals::{IntervalSet, UtcInterval};

/// Clips `coverage` to `bounds` and marks every instant no evidence covers as unknown, attributed
/// to `gap_origin`.
pub fn complete_unknown_gaps(
    bounds: UtcInterval,
    coverage: &ActivityCoverage,
    gap_origin: &AutomaticOrigin,
) -> ActivityCoverage {
    let evidence = ActivityCoverage::new(bounds, coverage.segments().iter().cloned());
    let covered: IntervalSet = evidence
        .segments()
        .iter()
        .map(|segment| segment.interval)
        .collect();
    let gaps = covered
        .iter()
        .fold(IntervalSet::new([bounds]), |gaps, &interval| {
            gaps.subtract(interval)
        });
    let gap_segments = gaps.iter().map(|&interval| {
        ActivitySegment::new(interval, ActivityState::Unknown, gap_origin.clone())
    });
    ActivityCoverage::new(
        bounds,
        evidence.segments().iter().cloned().chain(gap_segments),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::Timestamp;

    fn minute(minute: i64) -> Timestamp {
        Timestamp::from_second(1_788_253_200 + minute * 60).unwrap()
    }

    fn interval(start: i64, end: i64) -> UtcInterval {
        UtcInterval::new(minute(start), minute(end)).unwrap()
    }

    fn bounds() -> UtcInterval {
        interval(0, 240)
    }

    fn evidence_origin() -> AutomaticOrigin {
        AutomaticOrigin::new("activitywatch:test").unwrap()
    }

    fn gap_origin() -> AutomaticOrigin {
        AutomaticOrigin::new("activitywatch:coverage-gap").unwrap()
    }

    fn segment(start: i64, end: i64, state: ActivityState) -> ActivitySegment {
        ActivitySegment::new(interval(start, end), state, evidence_origin())
    }

    fn gap(start: i64, end: i64) -> ActivitySegment {
        ActivitySegment::new(interval(start, end), ActivityState::Unknown, gap_origin())
    }

    #[test]
    fn adds_leading_interior_and_trailing_unknown_gaps() {
        let coverage = ActivityCoverage::new(
            bounds(),
            [
                segment(30, 90, ActivityState::Active),
                segment(120, 180, ActivityState::Afk),
            ],
        );

        let completed = complete_unknown_gaps(bounds(), &coverage, &gap_origin());

        assert_eq!(
            completed.segments(),
            [
                gap(0, 30),
                segment(30, 90, ActivityState::Active),
                gap(90, 120),
                segment(120, 180, ActivityState::Afk),
                gap(180, 240),
            ]
        );
    }

    #[test]
    fn clips_evidence_and_uses_its_union_for_gaps() {
        let coverage = ActivityCoverage::new(
            interval(-60, 300),
            [
                segment(-30, 60, ActivityState::Active),
                segment(45, 150, ActivityState::Active),
            ],
        );

        let completed = complete_unknown_gaps(bounds(), &coverage, &gap_origin());

        assert_eq!(
            completed.segments(),
            [
                segment(0, 60, ActivityState::Active),
                segment(45, 150, ActivityState::Active),
                gap(150, 240),
            ]
        );
    }

    #[test]
    fn preserves_existing_unknown_evidence() {
        let malformed = ActivitySegment::new(
            interval(60, 90),
            ActivityState::Unknown,
            AutomaticOrigin::new("activitywatch:malformed").unwrap(),
        );
        let coverage = ActivityCoverage::new(
            bounds(),
            [segment(0, 60, ActivityState::Active), malformed.clone()],
        );

        let completed = complete_unknown_gaps(bounds(), &coverage, &gap_origin());

        assert_eq!(
            completed.segments(),
            [
                segment(0, 60, ActivityState::Active),
                malformed,
                gap(90, 240)
            ]
        );
    }

    #[test]
    fn makes_empty_coverage_entirely_unknown() {
        let completed = complete_unknown_gaps(
            bounds(),
            &ActivityCoverage::new(bounds(), []),
            &gap_origin(),
        );

        assert_eq!(completed.segments(), [gap(0, 240)]);
    }
}
