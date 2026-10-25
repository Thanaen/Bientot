use jiff::SignedDuration;

use super::{
    DailyAccountingCalculation, DailySummary, DailySummaryParts, IndicatorState, InvalidSummary,
};
use crate::credit::DailyCreditResult;
use crate::forecasting::{assess, indicator_state, project_departure};

/// The summary against the nominal target, without weekly credit.
pub fn build_nominal(
    calculation: &DailyAccountingCalculation,
) -> Result<DailySummary, InvalidSummary> {
    build(calculation, calculation.nominal_target(), None)
}

/// The summary against the effective target that `credit` leaves for the calculated day.
pub fn build_with_credit(
    calculation: &DailyAccountingCalculation,
    credit: DailyCreditResult,
) -> Result<DailySummary, InvalidSummary> {
    if credit.date() != calculation.workday().date {
        return Err(InvalidSummary::CreditForAnotherDay);
    }
    if credit.gross_credited_work() != calculation.accounting().credited_work() {
        return Err(InvalidSummary::CreditForOtherWork);
    }
    if credit.nominal_target() != calculation.nominal_target() {
        return Err(InvalidSummary::CreditForAnotherTarget);
    }
    build(calculation, credit.effective_target(), Some(credit))
}

fn build(
    calculation: &DailyAccountingCalculation,
    effective_target: SignedDuration,
    credit_result: Option<DailyCreditResult>,
) -> Result<DailySummary, InvalidSummary> {
    let credited_work = calculation.accounting().credited_work();
    let departure_projection = project_departure(
        calculation.projection_anchor(),
        credited_work,
        effective_target,
        calculation.lunch(),
    )
    .expect("accounted work is never negative and targets are positive");
    let indicator_state = if calculation.workday().is_working_day() {
        indicator_state(
            credited_work,
            effective_target,
            departure_projection.map(|projection| projection.projected_departure),
            calculation.preferred_latest_departure(),
            calculation.orange_warning_margin(),
        )
        .expect("the projection exists whenever the day is incomplete")
    } else if credited_work >= effective_target {
        IndicatorState::Blue
    } else {
        IndicatorState::Green
    };
    DailySummary::new(DailySummaryParts {
        computed_at: calculation.observed_at(),
        accounting: calculation.accounting().clone(),
        nominal_target: calculation.nominal_target(),
        effective_target,
        departure_projection,
        indicator_state,
        source_diagnostics: calculation.source_diagnostics().clone(),
        assessment: assess(calculation.accounting(), calculation.source_diagnostics()),
        credit_result,
    })
}

#[cfg(test)]
mod tests;
