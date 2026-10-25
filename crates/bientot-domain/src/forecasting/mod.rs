//! Forecasts for the running workday: departure, overtime, indicator colour and how far the
//! accounting can be trusted.

mod confidence;
mod departure;
mod indicator;
pub(crate) mod overtime;

pub use confidence::{AccountingAssessment, AssessmentReason, ForecastAvailability, assess};
pub use departure::{DepartureProjection, project_departure};
pub use indicator::indicator_state;
pub use overtime::overtime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidForecastInput {
    NegativeCreditedWork,
    NonPositiveTarget,
    NegativeWarningMargin,
    MissingDepartureProjection,
}

impl std::fmt::Display for InvalidForecastInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NegativeCreditedWork => "credited work cannot be negative",
            Self::NonPositiveTarget => "the effective target must be positive",
            Self::NegativeWarningMargin => "the warning margin cannot be negative",
            Self::MissingDepartureProjection => "an incomplete day requires a departure projection",
        })
    }
}

impl std::error::Error for InvalidForecastInput {}

fn check_work_and_target(
    credited_work: jiff::SignedDuration,
    effective_target: jiff::SignedDuration,
) -> Result<(), InvalidForecastInput> {
    if credited_work.is_negative() {
        return Err(InvalidForecastInput::NegativeCreditedWork);
    }
    if !effective_target.is_positive() {
        return Err(InvalidForecastInput::NonPositiveTarget);
    }
    Ok(())
}
