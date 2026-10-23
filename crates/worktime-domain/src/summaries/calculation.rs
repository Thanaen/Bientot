use jiff::{SignedDuration, Timestamp};

use crate::accounting::AccountingResult;
use crate::intervals::UtcInterval;
use crate::sources::SourceDiagnostics;
use crate::time::LocalWorkday;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidCalculation {
    NonPositiveNominalTarget,
    NegativeWarningMargin,
}

impl std::fmt::Display for InvalidCalculation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NonPositiveNominalTarget => "the nominal target must be positive",
            Self::NegativeWarningMargin => "the warning margin cannot be negative",
        })
    }
}

impl std::error::Error for InvalidCalculation {}

/// Everything a daily summary is built from. The departure is projected from the accounting
/// horizon, and the calculation is observed when its source diagnostics were.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DailyAccountingCalculation {
    accounting: AccountingResult,
    nominal_target: SignedDuration,
    lunch: UtcInterval,
    preferred_latest_departure: Timestamp,
    orange_warning_margin: SignedDuration,
    source_diagnostics: SourceDiagnostics,
}

impl DailyAccountingCalculation {
    pub fn new(
        accounting: AccountingResult,
        nominal_target: SignedDuration,
        lunch: UtcInterval,
        preferred_latest_departure: Timestamp,
        orange_warning_margin: SignedDuration,
        source_diagnostics: SourceDiagnostics,
    ) -> Result<Self, InvalidCalculation> {
        if !nominal_target.is_positive() {
            return Err(InvalidCalculation::NonPositiveNominalTarget);
        }
        if orange_warning_margin.is_negative() {
            return Err(InvalidCalculation::NegativeWarningMargin);
        }
        Ok(Self {
            accounting,
            nominal_target,
            lunch,
            preferred_latest_departure,
            orange_warning_margin,
            source_diagnostics,
        })
    }

    pub fn workday(&self) -> &LocalWorkday {
        &self.accounting.timeline().window().workday
    }

    pub fn observed_at(&self) -> Timestamp {
        self.source_diagnostics.observed_at()
    }

    pub fn projection_anchor(&self) -> Timestamp {
        self.accounting.timeline().through()
    }

    pub fn accounting(&self) -> &AccountingResult {
        &self.accounting
    }

    pub fn nominal_target(&self) -> SignedDuration {
        self.nominal_target
    }

    pub fn lunch(&self) -> UtcInterval {
        self.lunch
    }

    pub fn preferred_latest_departure(&self) -> Timestamp {
        self.preferred_latest_departure
    }

    pub fn orange_warning_margin(&self) -> SignedDuration {
        self.orange_warning_margin
    }

    pub fn source_diagnostics(&self) -> &SourceDiagnostics {
        &self.source_diagnostics
    }
}
