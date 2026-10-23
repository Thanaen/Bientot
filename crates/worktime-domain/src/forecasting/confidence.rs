use crate::accounting::{AccountingResult, SegmentKind};
use crate::sources::{SourceDiagnostics, SourceFreshness, SourceIntervalKind, SourceQuality};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ForecastAvailability {
    Confirmed,
    Estimated,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssessmentReason {
    PendingSettlement,
    InteriorCompatibilitySeam,
    HistoricalMissingCoverage,
    CurrentMissingCoverage,
    SourceUnavailable,
    SourceStale,
    SourceProtocolFailure,
    ProvisionalLockReview,
}

/// How far a forecast can be trusted, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountingAssessment {
    forecast: ForecastAvailability,
    reasons: Vec<AssessmentReason>,
}

impl AccountingAssessment {
    pub fn new(
        forecast: ForecastAvailability,
        reasons: impl IntoIterator<Item = AssessmentReason>,
    ) -> Self {
        let mut reasons: Vec<_> = reasons.into_iter().collect();
        reasons.sort_unstable();
        reasons.dedup();
        Self { forecast, reasons }
    }

    pub fn forecast(&self) -> ForecastAvailability {
        self.forecast
    }

    /// Distinct and sorted.
    pub fn reasons(&self) -> &[AssessmentReason] {
        &self.reasons
    }
}

/// Unknown time before the accounting horizon only makes the forecast an estimate; unknown time
/// at the horizon makes it unavailable unless the source is still settling that time.
pub fn assess(accounting: &AccountingResult, source: &SourceDiagnostics) -> AccountingAssessment {
    let mut reasons = source_reasons(source);
    if !accounting.pending_reviews().is_empty() {
        reasons.push(AssessmentReason::ProvisionalLockReview);
    }
    let timeline = accounting.timeline();
    let (current, historical): (Vec<_>, Vec<_>) = timeline
        .segments()
        .iter()
        .filter(|segment| *segment.kind() == SegmentKind::Unknown)
        .map(|segment| segment.interval())
        .partition(|interval| interval.end() == timeline.through());
    if current.is_empty() && historical.is_empty() {
        return AccountingAssessment::new(ForecastAvailability::Confirmed, reasons);
    }
    if !historical.is_empty() {
        reasons.push(AssessmentReason::HistoricalMissingCoverage);
    }
    if current.is_empty() {
        return AccountingAssessment::new(ForecastAvailability::Estimated, reasons);
    }
    let mut current_diagnostics = source.intervals().iter().filter(|finding| {
        current
            .iter()
            .any(|interval| interval.overlaps(finding.interval()))
    });
    let current_is_pending = current_diagnostics.clone().next().is_some()
        && current_diagnostics
            .all(|finding| finding.kind() == SourceIntervalKind::PendingSettlement);
    if current_is_pending {
        reasons.push(AssessmentReason::PendingSettlement);
        return AccountingAssessment::new(ForecastAvailability::Estimated, reasons);
    }
    reasons.push(AssessmentReason::CurrentMissingCoverage);
    AccountingAssessment::new(ForecastAvailability::Unavailable, reasons)
}

fn source_reasons(source: &SourceDiagnostics) -> Vec<AssessmentReason> {
    let operation = source.operation();
    let has_finding = |kind| {
        source
            .intervals()
            .iter()
            .any(|finding| finding.kind() == kind)
    };
    [
        (
            operation.freshness == SourceFreshness::Unavailable,
            AssessmentReason::SourceUnavailable,
        ),
        (
            operation.freshness == SourceFreshness::Stale,
            AssessmentReason::SourceStale,
        ),
        (
            operation.quality != SourceQuality::Complete,
            AssessmentReason::SourceProtocolFailure,
        ),
        (
            has_finding(SourceIntervalKind::PendingSettlement),
            AssessmentReason::PendingSettlement,
        ),
        (
            has_finding(SourceIntervalKind::InteriorCompatibilitySeam),
            AssessmentReason::InteriorCompatibilitySeam,
        ),
    ]
    .into_iter()
    .filter_map(|(applies, reason)| applies.then_some(reason))
    .collect()
}

#[cfg(test)]
mod tests;
