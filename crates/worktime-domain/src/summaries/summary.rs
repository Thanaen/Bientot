use jiff::{SignedDuration, Timestamp};

use super::IndicatorState;
use crate::accounting::{AccountingResult, SegmentKind};
use crate::credit::DailyCreditResult;
use crate::forecasting::overtime::excess;
use crate::forecasting::{AccountingAssessment, DepartureProjection, ForecastAvailability};
use crate::sources::{SourceDiagnostics, SourceHealth, SourceIntervalKind};
use crate::time::LocalWorkday;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidSummary {
    NonPositiveNominalTarget,
    EffectiveTargetOutOfRange,
    CreditResultMismatch,
    IndicatorContradictsCompletion,
    ProjectionContradictsCompletion,
    CreditForAnotherDay,
    CreditForOtherWork,
    CreditForAnotherTarget,
}

impl std::fmt::Display for InvalidSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NonPositiveNominalTarget => "the nominal target must be positive",
            Self::EffectiveTargetOutOfRange => {
                "the effective target must be positive and at most the nominal target"
            }
            Self::CreditResultMismatch => {
                "the credit result must match the summary workday, gross work, targets, and applied credit"
            }
            Self::IndicatorContradictsCompletion => {
                "blue must be used exactly when credited work reaches the effective target"
            }
            Self::ProjectionContradictsCompletion => {
                "incomplete days require a departure projection and complete days forbid one"
            }
            Self::CreditForAnotherDay => "the credit result must belong to the calculated workday",
            Self::CreditForOtherWork => {
                "the credit result must use the calculated gross credited work"
            }
            Self::CreditForAnotherTarget => {
                "the credit result must use the calculation nominal target"
            }
        })
    }
}

impl std::error::Error for InvalidSummary {}

/// The values a [`DailySummary`] is checked from. The workday is the accounting's, and the
/// applied carry credit is the gap between the nominal and effective targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DailySummaryParts {
    pub computed_at: Timestamp,
    pub accounting: AccountingResult,
    pub nominal_target: SignedDuration,
    pub effective_target: SignedDuration,
    pub departure_projection: Option<DepartureProjection>,
    pub indicator_state: IndicatorState,
    pub source_diagnostics: SourceDiagnostics,
    pub assessment: AccountingAssessment,
    pub credit_result: Option<DailyCreditResult>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DailySummary {
    parts: DailySummaryParts,
}

impl DailySummary {
    pub fn new(parts: DailySummaryParts) -> Result<Self, InvalidSummary> {
        if !parts.nominal_target.is_positive() {
            return Err(InvalidSummary::NonPositiveNominalTarget);
        }
        if !parts.effective_target.is_positive() || parts.effective_target > parts.nominal_target {
            return Err(InvalidSummary::EffectiveTargetOutOfRange);
        }
        let workday = &parts.accounting.timeline().window().workday;
        let applied_carry_credit = parts.nominal_target - parts.effective_target;
        if parts.credit_result.is_some_and(|credit| {
            credit.date() != workday.date
                || credit.gross_credited_work() != parts.accounting.credited_work()
                || credit.nominal_target() != parts.nominal_target
                || credit.effective_target() != parts.effective_target
                || credit.applied_credit() != applied_carry_credit
        }) {
            return Err(InvalidSummary::CreditResultMismatch);
        }
        let is_complete = parts.accounting.credited_work() >= parts.effective_target;
        if is_complete != (parts.indicator_state == IndicatorState::Blue) {
            return Err(InvalidSummary::IndicatorContradictsCompletion);
        }
        if is_complete == parts.departure_projection.is_some() {
            return Err(InvalidSummary::ProjectionContradictsCompletion);
        }
        Ok(Self { parts })
    }

    pub fn workday(&self) -> &LocalWorkday {
        &self.parts.accounting.timeline().window().workday
    }

    pub fn computed_at(&self) -> Timestamp {
        self.parts.computed_at
    }

    pub fn accounting(&self) -> &AccountingResult {
        &self.parts.accounting
    }

    pub fn credited_work(&self) -> SignedDuration {
        self.parts.accounting.credited_work()
    }

    pub fn nominal_target(&self) -> SignedDuration {
        self.parts.nominal_target
    }

    pub fn effective_target(&self) -> SignedDuration {
        self.parts.effective_target
    }

    pub fn applied_carry_credit(&self) -> SignedDuration {
        self.parts.nominal_target - self.parts.effective_target
    }

    pub fn credit_result(&self) -> Option<&DailyCreditResult> {
        self.parts.credit_result.as_ref()
    }

    pub fn departure_projection(&self) -> Option<DepartureProjection> {
        self.parts.departure_projection
    }

    pub fn indicator_state(&self) -> IndicatorState {
        self.parts.indicator_state
    }

    pub fn source_diagnostics(&self) -> &SourceDiagnostics {
        &self.parts.source_diagnostics
    }

    pub fn source_health(&self) -> &SourceHealth {
        self.parts.source_diagnostics.operation()
    }

    pub fn assessment(&self) -> &AccountingAssessment {
        &self.parts.assessment
    }

    pub fn can_show_forecast(&self) -> bool {
        self.parts.assessment.forecast() != ForecastAvailability::Unavailable
    }

    pub fn has_source_diagnostics(&self) -> bool {
        self.parts.source_diagnostics.has_findings()
    }

    /// The source is degraded, or unresolved source coverage is still unknown in the accounting.
    pub fn has_source_warning(&self) -> bool {
        let diagnostics = &self.parts.source_diagnostics;
        let segments = self.parts.accounting.timeline().segments();
        diagnostics.has_operational_issue()
            || diagnostics.intervals().iter().any(|finding| {
                finding.kind() == SourceIntervalKind::UnresolvedCoverage
                    && segments.iter().any(|segment| {
                        *segment.kind() == SegmentKind::Unknown
                            && segment.interval().overlaps(finding.interval())
                    })
            })
    }

    pub fn remaining_work(&self) -> SignedDuration {
        excess(self.parts.effective_target, self.credited_work())
    }

    pub fn credited_overtime(&self) -> SignedDuration {
        excess(self.credited_work(), self.parts.effective_target)
    }

    pub fn is_complete(&self) -> bool {
        self.credited_work() >= self.parts.effective_target
    }
}

#[cfg(test)]
mod tests;
