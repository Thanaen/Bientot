use jiff::SignedDuration;
use jiff::civil::{Date, Weekday};

use crate::configuration::WorkTimeSettings;
use crate::forecasting::overtime::excess;
use crate::time::days_after;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DailyCreditInput {
    pub date: Date,
    pub gross_credited_work: SignedDuration,
    /// Part of the gross work that a long-lock review may still withdraw.
    pub work_awaiting_review: SignedDuration,
    pub is_finished: bool,
}

impl DailyCreditInput {
    /// A finished day with nothing awaiting review.
    pub fn new(date: Date, gross_credited_work: SignedDuration) -> Self {
        Self {
            date,
            gross_credited_work,
            work_awaiting_review: SignedDuration::ZERO,
            is_finished: true,
        }
    }
}

/// One day of the weekly carry. Debt lives beside the credit lots, never inside them: it only
/// absorbs later surplus before that surplus is earned, so the lot ledger keeps its
/// earned, applied and expired shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DailyCreditResult {
    date: Date,
    gross_credited_work: SignedDuration,
    nominal_target: SignedDuration,
    opening_balance: SignedDuration,
    applied_credit: SignedDuration,
    earned_transferable_credit: SignedDuration,
    opening_debt: SignedDuration,
    repaid_debt: SignedDuration,
    incurred_debt: SignedDuration,
}

impl DailyCreditResult {
    pub fn date(&self) -> Date {
        self.date
    }

    pub fn gross_credited_work(&self) -> SignedDuration {
        self.gross_credited_work
    }

    pub fn nominal_target(&self) -> SignedDuration {
        self.nominal_target
    }

    pub fn opening_balance(&self) -> SignedDuration {
        self.opening_balance
    }

    pub fn applied_credit(&self) -> SignedDuration {
        self.applied_credit
    }

    pub fn effective_target(&self) -> SignedDuration {
        self.nominal_target - self.applied_credit
    }

    pub fn remaining_work(&self) -> SignedDuration {
        excess(self.effective_target(), self.gross_credited_work)
    }

    pub fn new_overtime(&self) -> SignedDuration {
        excess(self.gross_credited_work, self.effective_target())
    }

    pub fn earned_transferable_credit(&self) -> SignedDuration {
        self.earned_transferable_credit
    }

    pub fn closing_balance(&self) -> SignedDuration {
        self.opening_balance - self.applied_credit + self.earned_transferable_credit
    }

    pub fn credit_expiring_after_friday(&self) -> SignedDuration {
        if self.date.weekday() == Weekday::Friday {
            self.closing_balance()
        } else {
            SignedDuration::ZERO
        }
    }

    pub fn opening_debt(&self) -> SignedDuration {
        self.opening_debt
    }

    pub fn repaid_debt(&self) -> SignedDuration {
        self.repaid_debt
    }

    pub fn incurred_debt(&self) -> SignedDuration {
        self.incurred_debt
    }

    pub fn closing_debt(&self) -> SignedDuration {
        self.opening_debt - self.repaid_debt + self.incurred_debt
    }

    /// Available credit minus debt; negative when the week owes time.
    pub fn net_closing_balance(&self) -> SignedDuration {
        self.closing_balance() - self.closing_debt()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidWeeklyCredit {
    NominalTargetOutOfRange,
    MinimumTargetOutOfRange,
    WeekLength,
    NotStartingOnMonday,
    NotConsecutive,
    NegativeGrossWork,
    AwaitingReviewOutOfRange,
}

impl std::fmt::Display for InvalidWeeklyCredit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NominalTargetOutOfRange => "the nominal target must be between zero and 24 hours",
            Self::MinimumTargetOutOfRange => {
                "the minimum effective target must be positive and no greater than the nominal target"
            }
            Self::WeekLength => "a weekly calculation requires one to five workdays",
            Self::NotStartingOnMonday => "a weekly calculation must begin on Monday",
            Self::NotConsecutive => "daily credit inputs must be a consecutive Monday-to-Friday prefix",
            Self::NegativeGrossWork => "gross credited work cannot be negative",
            Self::AwaitingReviewOutOfRange => {
                "work awaiting review must be between zero and gross credited work"
            }
        })
    }
}

impl std::error::Error for InvalidWeeklyCredit {}

/// Carries each day's surplus over the nominal target to later days of the same week, lowering
/// their target down to the minimum effective target. With debt enabled, a finished day under
/// its target leaves a debt that later surplus repays before it becomes credit.
pub fn calculate_week(
    inputs: &[DailyCreditInput],
    settings: &WorkTimeSettings,
) -> Result<Vec<DailyCreditResult>, InvalidWeeklyCredit> {
    validate_settings(settings)?;
    validate_inputs(inputs)?;

    let credit_enabled = settings.weekly_credit_enabled;
    let debt_enabled = credit_enabled && settings.weekly_debt_enabled;
    let nominal_target = settings.nominal_daily_target;
    let maximum_daily_credit = nominal_target - settings.minimum_effective_target;
    let mut balance = SignedDuration::ZERO;
    let mut debt = SignedDuration::ZERO;
    let mut results = Vec::with_capacity(inputs.len());
    for input in inputs {
        let applied_credit = if credit_enabled {
            balance.min(maximum_daily_credit)
        } else {
            SignedDuration::ZERO
        };
        let effective_target = nominal_target - applied_credit;
        let surplus = if credit_enabled {
            excess(
                input.gross_credited_work - input.work_awaiting_review,
                effective_target,
            )
        } else {
            SignedDuration::ZERO
        };
        let mut repaid_debt = SignedDuration::ZERO;
        let mut incurred_debt = SignedDuration::ZERO;
        if debt_enabled {
            // Work awaiting review cannot repay debt, but it still counts toward the target.
            repaid_debt = debt.min(surplus);
            if input.is_finished
                && input.gross_credited_work.is_positive()
                && input.gross_credited_work < effective_target
            {
                incurred_debt = effective_target - input.gross_credited_work;
                if let Some(maximum_debt) = settings.maximum_weekly_debt {
                    incurred_debt = incurred_debt
                        .min((maximum_debt - (debt - repaid_debt)).max(SignedDuration::ZERO));
                }
            }
        }
        let result = DailyCreditResult {
            date: input.date,
            gross_credited_work: input.gross_credited_work,
            nominal_target,
            opening_balance: balance,
            applied_credit,
            earned_transferable_credit: surplus - repaid_debt,
            opening_debt: debt,
            repaid_debt,
            incurred_debt,
        };
        balance = result.closing_balance();
        debt = result.closing_debt();
        results.push(result);
    }
    Ok(results)
}

fn validate_settings(settings: &WorkTimeSettings) -> Result<(), InvalidWeeklyCredit> {
    let nominal = settings.nominal_daily_target;
    if !nominal.is_positive() || nominal > SignedDuration::from_hours(24) {
        return Err(InvalidWeeklyCredit::NominalTargetOutOfRange);
    }
    let minimum = settings.minimum_effective_target;
    if !minimum.is_positive() || minimum > nominal {
        return Err(InvalidWeeklyCredit::MinimumTargetOutOfRange);
    }
    Ok(())
}

fn validate_inputs(inputs: &[DailyCreditInput]) -> Result<(), InvalidWeeklyCredit> {
    let Some(monday) = inputs.first().filter(|_| inputs.len() <= 5) else {
        return Err(InvalidWeeklyCredit::WeekLength);
    };
    if monday.date.weekday() != Weekday::Monday {
        return Err(InvalidWeeklyCredit::NotStartingOnMonday);
    }
    for (offset, input) in (0..).zip(inputs) {
        if input.date != days_after(monday.date, offset) {
            return Err(InvalidWeeklyCredit::NotConsecutive);
        }
        if input.gross_credited_work.is_negative() {
            return Err(InvalidWeeklyCredit::NegativeGrossWork);
        }
        if input.work_awaiting_review.is_negative()
            || input.work_awaiting_review > input.gross_credited_work
        {
            return Err(InvalidWeeklyCredit::AwaitingReviewOutOfRange);
        }
    }
    Ok(())
}
