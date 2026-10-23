use jiff::Timestamp;

use super::SourceHealth;
use crate::accounting::BlankSourceId;
use crate::intervals::UtcInterval;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceIntervalKind {
    PendingSettlement,
    InteriorCompatibilitySeam,
    UnresolvedCoverage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDiagnosticInterval {
    interval: UtcInterval,
    kind: SourceIntervalKind,
    source_id: String,
}

impl SourceDiagnosticInterval {
    pub fn new(
        interval: UtcInterval,
        kind: SourceIntervalKind,
        source_id: &str,
    ) -> Result<Self, BlankSourceId> {
        let source_id = source_id.trim();
        if source_id.is_empty() {
            return Err(BlankSourceId);
        }
        Ok(Self {
            interval,
            kind,
            source_id: source_id.to_owned(),
        })
    }

    pub fn interval(&self) -> UtcInterval {
        self.interval
    }

    pub fn kind(&self) -> SourceIntervalKind {
        self.kind
    }

    pub fn source_id(&self) -> &str {
        &self.source_id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidDiagnostics {
    OutsideRequestBounds,
    Overlapping,
}

impl std::fmt::Display for InvalidDiagnostics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::OutsideRequestBounds => {
                "source diagnostic intervals must lie inside the request bounds"
            }
            Self::Overlapping => "source diagnostic intervals must not overlap",
        })
    }
}

impl std::error::Error for InvalidDiagnostics {}

/// What one source query found: its operational health and the intervals it could not settle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDiagnostics {
    operation: SourceHealth,
    request_bounds: UtcInterval,
    observed_at: Timestamp,
    intervals: Vec<SourceDiagnosticInterval>,
}

impl SourceDiagnostics {
    pub fn new(
        operation: SourceHealth,
        request_bounds: UtcInterval,
        observed_at: Timestamp,
        intervals: impl IntoIterator<Item = SourceDiagnosticInterval>,
    ) -> Result<Self, InvalidDiagnostics> {
        let mut intervals: Vec<_> = intervals.into_iter().collect();
        intervals.sort_by_key(|interval| (interval.interval.start(), interval.interval.end()));
        if intervals
            .iter()
            .any(|interval| interval.interval.intersect(request_bounds) != Some(interval.interval))
        {
            return Err(InvalidDiagnostics::OutsideRequestBounds);
        }
        if intervals
            .windows(2)
            .any(|pair| pair[0].interval.overlaps(pair[1].interval))
        {
            return Err(InvalidDiagnostics::Overlapping);
        }
        Ok(Self {
            operation,
            request_bounds,
            observed_at,
            intervals,
        })
    }

    pub fn operation(&self) -> &SourceHealth {
        &self.operation
    }

    pub fn request_bounds(&self) -> UtcInterval {
        self.request_bounds
    }

    pub fn observed_at(&self) -> Timestamp {
        self.observed_at
    }

    pub fn intervals(&self) -> &[SourceDiagnosticInterval] {
        &self.intervals
    }

    pub fn has_operational_issue(&self) -> bool {
        self.operation.is_degraded()
    }

    pub fn has_findings(&self) -> bool {
        self.has_operational_issue() || !self.intervals.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minute(minute: i64) -> Timestamp {
        Timestamp::from_second(1_788_000_000 + minute * 60).unwrap()
    }

    fn interval(start: i64, end: i64) -> UtcInterval {
        UtcInterval::new(minute(start), minute(end)).unwrap()
    }

    fn finding(start: i64, end: i64) -> SourceDiagnosticInterval {
        SourceDiagnosticInterval::new(
            interval(start, end),
            SourceIntervalKind::UnresolvedCoverage,
            " builtin ",
        )
        .unwrap()
    }

    fn diagnostics(
        findings: Vec<SourceDiagnosticInterval>,
    ) -> Result<SourceDiagnostics, InvalidDiagnostics> {
        SourceDiagnostics::new(
            SourceHealth::healthy("builtin", minute(60)),
            interval(0, 60),
            minute(60),
            findings,
        )
    }

    #[test]
    fn findings_are_sorted_and_their_source_trimmed() {
        let diagnostics = diagnostics(vec![finding(30, 40), finding(0, 10)]).unwrap();

        let starts: Vec<_> = diagnostics
            .intervals()
            .iter()
            .map(|finding| finding.interval())
            .collect();
        assert_eq!(starts, [interval(0, 10), interval(30, 40)]);
        assert_eq!(diagnostics.intervals()[0].source_id(), "builtin");
        assert!(!diagnostics.has_operational_issue());
        assert!(diagnostics.has_findings());
    }

    #[test]
    fn findings_outside_the_request_or_overlapping_are_rejected() {
        assert_eq!(
            diagnostics(vec![finding(50, 70)]),
            Err(InvalidDiagnostics::OutsideRequestBounds)
        );
        assert_eq!(
            diagnostics(vec![finding(0, 20), finding(10, 30)]),
            Err(InvalidDiagnostics::Overlapping)
        );
        assert!(diagnostics(vec![finding(0, 10), finding(10, 20)]).is_ok());
    }

    #[test]
    fn a_healthy_source_without_findings_reports_nothing() {
        assert!(!diagnostics(vec![]).unwrap().has_findings());
    }
}
