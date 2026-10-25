//! The daily summary: accounting, targets, forecast and source state of one workday.

mod builder;
mod calculation;
mod summary;

pub use builder::{build_nominal, build_with_credit};
pub use calculation::{DailyAccountingCalculation, InvalidCalculation};
pub use summary::{DailySummary, DailySummaryParts, InvalidSummary};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IndicatorState {
    Green,
    Orange,
    Red,
    Blue,
}
