use std::collections::{BTreeMap, HashMap, VecDeque};

use jiff::civil::{Date, Weekday};
use jiff::{SignedDuration, Timestamp};

use super::entry::{CreditLedgerEntry, CreditLedgerEntryId, CreditLedgerEntryKind, CreditOrigin};
use super::is_workday;
use super::policy::DailyCreditResult;
use crate::time::WorkdayWindow;
use crate::time::days_after;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidLedgerInput {
    WeekLength,
    NotStartingOnMonday,
    NotConsecutive,
    MissingRevision,
    RevisionOutsideWeek,
    InvalidFridayWindow,
    PreviousEntryOutsideWeek,
    DuplicatePreviousEntry,
    OpeningBalanceMismatch,
    AppliedCreditUnavailable,
    NegativeEarnedCredit,
    ClosingBalanceMismatch,
}

impl std::fmt::Display for InvalidLedgerInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::WeekLength => "a weekly ledger requires a one-to-five-day policy prefix",
            Self::NotStartingOnMonday => "a weekly ledger prefix must begin on Monday",
            Self::NotConsecutive => {
                "daily credit results must be a consecutive Monday-to-Friday prefix"
            }
            Self::MissingRevision => "every supplied day requires a positive durable revision",
            Self::RevisionOutsideWeek => {
                "durable revisions must be positive and belong to the selected workweek"
            }
            Self::InvalidFridayWindow => {
                "the Friday window must be a nonempty window for Friday in the selected week"
            }
            Self::PreviousEntryOutsideWeek => {
                "previous ledger entries must belong to the selected week"
            }
            Self::DuplicatePreviousEntry => "previous ledger entry IDs must be unique",
            Self::OpeningBalanceMismatch => {
                "daily credit opening balances must match the preceding durable lots"
            }
            Self::AppliedCreditUnavailable => {
                "applied credit must be a nonnegative amount available from prior earned lots"
            }
            Self::NegativeEarnedCredit => "earned transferable credit cannot be negative",
            Self::ClosingBalanceMismatch => {
                "daily credit closing balances must conserve applied and earned lots exactly"
            }
        })
    }
}

impl std::error::Error for InvalidLedgerInput {}

struct CreditLot {
    origin: CreditOrigin,
    remaining: SignedDuration,
}

/// The movements of the week's credit lots, applied oldest lot first, with Friday's leftover
/// expiring once `observed_at` reaches the end of Friday. Movement IDs are deterministic, and a
/// movement already in `previous_ledger` keeps its creation instant.
pub fn build_ledger(
    daily_results: &[DailyCreditResult],
    durable_revisions: &BTreeMap<Date, i64>,
    friday_window: &WorkdayWindow,
    observed_at: Timestamp,
    previous_ledger: &[CreditLedgerEntry],
) -> Result<Vec<CreditLedgerEntry>, InvalidLedgerInput> {
    let week_start = validate_inputs(daily_results, durable_revisions, friday_window)?;
    let movements = Movements {
        week_start,
        observed_at,
        previous: index_previous(previous_ledger, week_start)?,
    };
    let mut entries = Vec::new();
    let mut lots: VecDeque<CreditLot> = VecDeque::new();
    let mut available = SignedDuration::ZERO;
    for result in daily_results {
        let today = CreditOrigin {
            workday_date: result.date(),
            revision: durable_revisions[&result.date()],
        };
        if result.opening_balance() != available {
            return Err(InvalidLedgerInput::OpeningBalanceMismatch);
        }
        let applied = result.applied_credit();
        if applied.is_negative() || applied > available {
            return Err(InvalidLedgerInput::AppliedCreditUnavailable);
        }
        let mut remaining_applied = applied;
        while remaining_applied.is_positive() {
            let lot = lots
                .front_mut()
                .expect("applied credit never exceeds the open lots");
            let consumed = remaining_applied.min(lot.remaining);
            entries.push(movements.entry(
                today,
                CreditLedgerEntryKind::Applied,
                consumed,
                lot.origin,
            ));
            lot.remaining -= consumed;
            remaining_applied -= consumed;
            available -= consumed;
            if lot.remaining.is_zero() {
                lots.pop_front();
            }
        }
        let earned = result.earned_transferable_credit();
        if earned.is_negative() {
            return Err(InvalidLedgerInput::NegativeEarnedCredit);
        }
        if earned.is_positive() {
            entries.push(movements.entry(today, CreditLedgerEntryKind::Earned, earned, today));
            lots.push_back(CreditLot {
                origin: today,
                remaining: earned,
            });
            available += earned;
        }
        if result.closing_balance() != available {
            return Err(InvalidLedgerInput::ClosingBalanceMismatch);
        }
    }
    let friday = friday_window.workday.date;
    let reached_friday = daily_results.last().map(DailyCreditResult::date) == Some(friday);
    if reached_friday && observed_at >= friday_window.end {
        let friday = CreditOrigin {
            workday_date: friday,
            revision: durable_revisions[&friday],
        };
        for lot in lots {
            entries.push(movements.entry(
                friday,
                CreditLedgerEntryKind::Expired,
                lot.remaining,
                lot.origin,
            ));
        }
    }
    Ok(entries)
}

fn validate_inputs(
    daily_results: &[DailyCreditResult],
    durable_revisions: &BTreeMap<Date, i64>,
    friday_window: &WorkdayWindow,
) -> Result<Date, InvalidLedgerInput> {
    let Some(monday) = daily_results.first().filter(|_| daily_results.len() <= 5) else {
        return Err(InvalidLedgerInput::WeekLength);
    };
    let week_start = monday.date();
    if week_start.weekday() != Weekday::Monday {
        return Err(InvalidLedgerInput::NotStartingOnMonday);
    }
    let friday = days_after(week_start, 4);
    for (offset, result) in (0..).zip(daily_results) {
        if result.date() != days_after(week_start, offset) {
            return Err(InvalidLedgerInput::NotConsecutive);
        }
        if durable_revisions
            .get(&result.date())
            .is_none_or(|&revision| revision <= 0)
        {
            return Err(InvalidLedgerInput::MissingRevision);
        }
    }
    if durable_revisions.iter().any(|(&date, &revision)| {
        revision <= 0 || date < week_start || date > friday || !is_workday(date)
    }) {
        return Err(InvalidLedgerInput::RevisionOutsideWeek);
    }
    if friday_window.workday.time_zone_id.trim().is_empty()
        || friday_window.workday.date != friday
        || friday_window.end <= friday_window.start
    {
        return Err(InvalidLedgerInput::InvalidFridayWindow);
    }
    Ok(week_start)
}

fn index_previous(
    previous_ledger: &[CreditLedgerEntry],
    week_start: Date,
) -> Result<HashMap<&str, &CreditLedgerEntry>, InvalidLedgerInput> {
    let mut by_id = HashMap::with_capacity(previous_ledger.len());
    for entry in previous_ledger {
        if entry.week_start_date() != week_start {
            return Err(InvalidLedgerInput::PreviousEntryOutsideWeek);
        }
        if by_id.insert(entry.id().as_str(), entry).is_some() {
            return Err(InvalidLedgerInput::DuplicatePreviousEntry);
        }
    }
    Ok(by_id)
}

struct Movements<'a> {
    week_start: Date,
    observed_at: Timestamp,
    previous: HashMap<&'a str, &'a CreditLedgerEntry>,
}

impl Movements<'_> {
    fn entry(
        &self,
        movement: CreditOrigin,
        kind: CreditLedgerEntryKind,
        amount: SignedDuration,
        origin: CreditOrigin,
    ) -> CreditLedgerEntry {
        let id = format!(
            "weekly-credit:v1:{}:{}:r{}:{}:t{}:{}:r{}",
            compact(self.week_start),
            compact(movement.workday_date),
            movement.revision,
            id_part(kind),
            ticks(amount),
            compact(origin.workday_date),
            origin.revision,
        );
        let created_at = self
            .previous
            .get(id.as_str())
            .filter(|previous| {
                previous.workday_date() == movement.workday_date
                    && previous.kind() == kind
                    && ticks(previous.amount()) == ticks(amount)
                    && previous.origin() == origin
            })
            .map_or(self.observed_at, |previous| previous.created_at());
        CreditLedgerEntry::new(
            CreditLedgerEntryId::new(id).expect("movement IDs are never blank"),
            movement.workday_date,
            kind,
            amount,
            origin,
            created_at,
        )
        .expect("validated inputs produce valid movements")
    }
}

fn compact(date: Date) -> String {
    date.strftime("%Y%m%d").to_string()
}

/// .NET `TimeSpan` ticks of 100 ns, which the persisted movement IDs are written in.
fn ticks(duration: SignedDuration) -> i128 {
    duration.as_nanos() / 100
}

fn id_part(kind: CreditLedgerEntryKind) -> &'static str {
    match kind {
        CreditLedgerEntryKind::Earned => "earned",
        CreditLedgerEntryKind::Applied => "applied",
        CreditLedgerEntryKind::Expired => "expired",
        CreditLedgerEntryKind::Reversed => unreachable!("the builder never reverses a movement"),
    }
}
