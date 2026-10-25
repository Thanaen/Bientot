//! Weekly carry of overtime: the per-day policy and the durable ledger of credit lots it implies.

mod entry;
mod ledger;
mod policy;

pub use entry::{
    CreditLedgerEntry, CreditLedgerEntryId, CreditLedgerEntryKind, CreditOrigin, InvalidLedgerEntry,
};
pub use ledger::{InvalidLedgerInput, build_ledger};
pub use policy::{DailyCreditInput, DailyCreditResult, InvalidWeeklyCredit, calculate_week};

use jiff::civil::{Date, Weekday};

fn is_workday(date: Date) -> bool {
    !matches!(date.weekday(), Weekday::Saturday | Weekday::Sunday)
}

#[cfg(test)]
mod tests;
