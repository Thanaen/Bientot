//! A week of daily summaries and its credit ledger, committed together, ported from
//! `Credit/WeeklyCreditRepository.cs` and the weekly types of
//! `App/Abstractions/Persistence/IWeeklyCreditStore.cs`.

use std::collections::{BTreeMap, HashSet};

use bientot_domain::credit::{
    CreditLedgerEntry, CreditLedgerEntryId, CreditLedgerEntryKind, CreditOrigin,
};
use bientot_domain::history::DailySummaryRevision;
use jiff::civil::{Date, Weekday};
use rusqlite::{Connection, TransactionBehavior, named_params};

use super::{CreditLedgerError, LedgerRecord, read_week, replace_week, replacement_records};
use crate::codec::{DurationOutOfRange, format_date};
use crate::history::{self, DailySummaryError, DailySummaryRepository, weekdays};
use crate::row::{InvalidStoredValue, StorageError, StoredRow, collect, storage_error_conversions};

/// The latest revision of each weekday when a weekly recalculation started, 0 for a day without
/// a summary. A commit goes through only if the week still has these revisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeeklyRevisionVector {
    week_start_date: Date,
    revisions: [i64; 5],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidRevisionVector {
    NotAMonday,
    NegativeRevision,
}

impl std::fmt::Display for InvalidRevisionVector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotAMonday => "a weekly revision vector must start on Monday",
            Self::NegativeRevision => "latest revisions cannot be negative",
        })
    }
}

impl std::error::Error for InvalidRevisionVector {}

impl WeeklyRevisionVector {
    /// `revisions` holds Monday to Friday.
    pub fn new(week_start_date: Date, revisions: [i64; 5]) -> Result<Self, InvalidRevisionVector> {
        if week_start_date.weekday() != Weekday::Monday {
            return Err(InvalidRevisionVector::NotAMonday);
        }
        if revisions.iter().any(|revision| *revision < 0) {
            return Err(InvalidRevisionVector::NegativeRevision);
        }
        Ok(Self {
            week_start_date,
            revisions,
        })
    }

    pub fn week_start_date(&self) -> Date {
        self.week_start_date
    }

    /// Monday to Friday.
    pub fn revisions(&self) -> [i64; 5] {
        self.revisions
    }

    /// The revision of `workday_date`, `None` outside Monday to Friday of this week.
    pub fn revision(&self, workday_date: Date) -> Option<i64> {
        weekdays(self.week_start_date)
            .iter()
            .position(|day| *day == workday_date)
            .map(|offset| self.revisions[offset])
    }
}

/// The latest summary of each weekday that has one, by date, with the week's ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeeklyCreditState {
    week_start_date: Date,
    latest_revisions: Vec<DailySummaryRevision>,
    ledger: Vec<CreditLedgerEntry>,
}

impl WeeklyCreditState {
    pub fn week_start_date(&self) -> Date {
        self.week_start_date
    }

    pub fn latest_revisions(&self) -> &[DailySummaryRevision] {
        &self.latest_revisions
    }

    pub fn ledger(&self) -> &[CreditLedgerEntry] {
        &self.ledger
    }

    pub fn revision_vector(&self) -> WeeklyRevisionVector {
        let mut revisions = [0; 5];
        for (day, revision) in weekdays(self.week_start_date).iter().zip(&mut revisions) {
            if let Some(latest) = self
                .latest_revisions
                .iter()
                .find(|latest| latest.workday_date() == *day)
            {
                *revision = latest.revision();
            }
        }
        WeeklyRevisionVector {
            week_start_date: self.week_start_date,
            revisions,
        }
    }
}

/// Why a weekly ledger disagrees with the summaries it would be committed with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WeeklyInconsistency {
    ReversedMovement,
    DayWithoutSummary(CreditLedgerEntryId),
    /// The movement's origin is not the latest summary revision of its origin day.
    UnselectedOrigin(CreditLedgerEntryId),
    EarnedFromAnotherDay(CreditLedgerEntryId),
    /// An applied movement draws on a lot earned the same day or later.
    AppliedFromLaterOrigin(CreditLedgerEntryId),
    ExpiryNotOnFriday,
    /// The movements of one kind on a day do not add up to the summary's carry column.
    TotalMismatch {
        workday_date: Date,
        kind: CreditLedgerEntryKind,
        ledger_ticks: i64,
        summary_ticks: i64,
    },
    TotalsOverflow,
    ExceedsEarnedLots {
        workday_date: Date,
        kind: CreditLedgerEntryKind,
    },
    /// The movements consume earned lots in another order than oldest first.
    NotFifo {
        workday_date: Date,
        kind: CreditLedgerEntryKind,
    },
    /// Friday expiry leaves earned credit behind.
    PartialExpiry,
    AvailableMismatch {
        workday_date: Date,
        ledger_ticks: i64,
        summary_ticks: i64,
    },
}

fn kind_name(kind: CreditLedgerEntryKind) -> &'static str {
    match kind {
        CreditLedgerEntryKind::Earned => "earned",
        CreditLedgerEntryKind::Applied => "applied",
        CreditLedgerEntryKind::Expired => "expired",
        CreditLedgerEntryKind::Reversed => "reversed",
    }
}

impl std::fmt::Display for WeeklyInconsistency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReversedMovement => {
                f.write_str("A canonical weekly replacement cannot contain reversed movements.")
            }
            Self::DayWithoutSummary(id) => write!(
                f,
                "Credit movement {id} belongs to a day without a durable summary."
            ),
            Self::UnselectedOrigin(id) => write!(
                f,
                "Credit movement {id} does not reference the selected durable origin revision."
            ),
            Self::EarnedFromAnotherDay(id) => write!(
                f,
                "Earned movement {id} must originate from its own workday."
            ),
            Self::AppliedFromLaterOrigin(id) => write!(
                f,
                "Applied movement {id} must consume an earlier earned lot."
            ),
            Self::ExpiryNotOnFriday => f.write_str("Credit can expire only on Friday."),
            Self::TotalMismatch {
                workday_date,
                kind,
                ledger_ticks,
                summary_ticks,
            } => write!(
                f,
                "The {} ledger total for {} is {ledger_ticks} ticks; the selected summary stores \
                 {summary_ticks} ticks.",
                kind_name(*kind),
                format_date(*workday_date)
            ),
            Self::TotalsOverflow => {
                f.write_str("Credit ledger totals exceed the supported duration range.")
            }
            Self::ExceedsEarnedLots { workday_date, kind } => write!(
                f,
                "The {} credit for {} exceeds the available earned lots.",
                kind_name(*kind),
                format_date(*workday_date)
            ),
            Self::NotFifo { workday_date, kind } => write!(
                f,
                "The {} credit for {} does not consume earned lots FIFO.",
                kind_name(*kind),
                format_date(*workday_date)
            ),
            Self::PartialExpiry => {
                f.write_str("Friday expiry must consume every remaining earned lot.")
            }
            Self::AvailableMismatch {
                workday_date,
                ledger_ticks,
                summary_ticks,
            } => write!(
                f,
                "The available credit for {} is {ledger_ticks} ticks; the selected summary \
                 stores {summary_ticks} ticks.",
                format_date(*workday_date)
            ),
        }
    }
}

#[derive(Debug)]
pub enum WeeklyCreditError {
    Storage(StorageError),
    /// A week starts on Monday.
    NotAMonday(Date),
    /// The expected revisions belong to another week than the committed one.
    VectorOfAnotherWeek(Date),
    RevisionOutsideWeek {
        workday_date: Date,
        revision: i64,
    },
    DuplicateRevision {
        workday_date: Date,
        revision: i64,
    },
    EntryOutsideWeek(CreditLedgerEntryId),
    DurationOutOfRange(DurationOutOfRange),
    /// A summary revision could not be appended.
    Summary(DailySummaryError),
    /// The week changed since its revisions were read.
    Stale {
        expected: WeeklyRevisionVector,
        actual: WeeklyRevisionVector,
    },
    Inconsistent(WeeklyInconsistency),
}

impl std::fmt::Display for WeeklyCreditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(f),
            Self::NotAMonday(date) => {
                write!(
                    f,
                    "A week must start on Monday, not {}.",
                    format_date(*date)
                )
            }
            Self::VectorOfAnotherWeek(date) => write!(
                f,
                "The expected revision vector of the week of {} must belong to the committed week.",
                format_date(*date)
            ),
            Self::RevisionOutsideWeek {
                workday_date,
                revision,
            } => write!(
                f,
                "Daily summary revision {revision} for {} does not belong to the committed week.",
                format_date(*workday_date)
            ),
            Self::DuplicateRevision {
                workday_date,
                revision,
            } => write!(
                f,
                "A weekly commit cannot contain daily summary revision {revision} for {} twice.",
                format_date(*workday_date)
            ),
            Self::EntryOutsideWeek(id) => write!(
                f,
                "Credit ledger entry {id} does not belong to the committed week."
            ),
            Self::DurationOutOfRange(error) => error.fmt(f),
            Self::Summary(error) => error.fmt(f),
            Self::Stale { expected, .. } => write!(
                f,
                "Weekly credit state for {} changed before commit.",
                format_date(expected.week_start_date)
            ),
            Self::Inconsistent(inconsistency) => inconsistency.fmt(f),
        }
    }
}

impl std::error::Error for WeeklyCreditError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::DurationOutOfRange(error) => Some(error),
            Self::Summary(error) => Some(error),
            _ => None,
        }
    }
}

storage_error_conversions!(WeeklyCreditError);

impl From<CreditLedgerError> for WeeklyCreditError {
    fn from(error: CreditLedgerError) -> Self {
        match error {
            CreditLedgerError::Storage(error) => Self::Storage(error),
            CreditLedgerError::NotAMonday(date) => Self::NotAMonday(date),
            CreditLedgerError::EntryOutsideWeek(id) => Self::EntryOutsideWeek(id),
            CreditLedgerError::DurationOutOfRange(error) => Self::DurationOutOfRange(error),
        }
    }
}

impl From<DailySummaryError> for WeeklyCreditError {
    fn from(error: DailySummaryError) -> Self {
        match error {
            DailySummaryError::Storage(error) => Self::Storage(error),
            DailySummaryError::DurationOutOfRange(error) => Self::DurationOutOfRange(error),
            error => Self::Summary(error),
        }
    }
}

fn require_monday(date: Date) -> Result<(), WeeklyCreditError> {
    super::require_monday(date).map_err(WeeklyCreditError::from)
}

/// The carry columns of a day's latest summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Head {
    revision: i64,
    carry_available_ticks: i64,
    carry_earned_ticks: i64,
    carry_applied_ticks: i64,
    carry_expired_ticks: i64,
}

impl Head {
    fn total(&self, kind: CreditLedgerEntryKind) -> i64 {
        match kind {
            CreditLedgerEntryKind::Earned => self.carry_earned_ticks,
            CreditLedgerEntryKind::Applied => self.carry_applied_ticks,
            CreditLedgerEntryKind::Expired => self.carry_expired_ticks,
            CreditLedgerEntryKind::Reversed => unreachable!("reversed movements are refused first"),
        }
    }
}

pub struct WeeklyCreditRepository<'a> {
    connection: &'a Connection,
}

impl<'a> WeeklyCreditRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// The weeks before `before_monday` whose last summarized weekday still holds credit or
    /// pending debt, oldest first: the weeks whose Friday expiry is still to be committed.
    pub fn read_pending_expiry_weeks(
        &self,
        before_monday: Date,
    ) -> Result<Vec<Date>, WeeklyCreditError> {
        require_monday(before_monday)?;
        let mut statement = self.connection.prepare(
            "WITH LatestDailyHeads AS (
    SELECT
        current.WorkdayDate,
        current.CarryAvailableTicks,
        current.DebtPending
    FROM DailySummaryRevision AS current
    WHERE current.Revision = (
        SELECT MAX(candidate.Revision)
        FROM DailySummaryRevision AS candidate
        WHERE candidate.WorkdayDate = current.WorkdayDate)
      AND CAST(strftime('%w', current.WorkdayDate) AS INTEGER) BETWEEN 1 AND 5
), WeekdayHeads AS (
    SELECT
        date(
            WorkdayDate,
            printf('-%d days', CAST(strftime('%w', WorkdayDate) AS INTEGER) - 1)
        ) AS WeekStartDate,
        WorkdayDate,
        CarryAvailableTicks,
        DebtPending,
        ROW_NUMBER() OVER (
            PARTITION BY date(
                WorkdayDate,
                printf('-%d days', CAST(strftime('%w', WorkdayDate) AS INTEGER) - 1)
            )
            ORDER BY WorkdayDate DESC
        ) AS ObservedOrder
    FROM LatestDailyHeads
)
SELECT WeekStartDate
FROM WeekdayHeads
WHERE ObservedOrder = 1
  AND (CarryAvailableTicks > 0 OR DebtPending = 1)
  AND WeekStartDate < $beforeWeekStartDate
ORDER BY WeekStartDate;",
        )?;
        let rows = statement.query_map(
            named_params! { "$beforeWeekStartDate": format_date(before_monday) },
            |row| Ok(StoredRow::new(row, history::TABLE).date("WeekStartDate")),
        )?;
        collect(rows)
    }

    /// The latest summaries and the ledger of the week starting on `monday`, read together.
    pub fn read_week(&self, monday: Date) -> Result<WeeklyCreditState, WeeklyCreditError> {
        require_monday(monday)?;
        let transaction = crate::database::scope(self.connection, TransactionBehavior::Deferred)?;
        let latest_revisions = history::read_latest_week(&transaction, monday)?;
        let ledger = read_week(&transaction, monday)?;
        transaction.commit()?;
        Ok(WeeklyCreditState {
            week_start_date: monday,
            latest_revisions,
            ledger,
        })
    }

    pub fn read_revision_vector(
        &self,
        monday: Date,
    ) -> Result<WeeklyRevisionVector, WeeklyCreditError> {
        require_monday(monday)?;
        read_revision_vector(self.connection, monday)
    }

    /// Appends `revisions_to_append` and replaces the week's ledger with `replacement_ledger`, at
    /// once, if the week still has the `expected_revisions` and the ledger agrees with the
    /// resulting summaries: FIFO lots, expiry on Friday only and of every remaining lot, and
    /// per-day totals equal to the summaries' carry columns.
    pub fn commit(
        &self,
        monday: Date,
        expected_revisions: &WeeklyRevisionVector,
        revisions_to_append: &[DailySummaryRevision],
        replacement_ledger: &[CreditLedgerEntry],
    ) -> Result<(), WeeklyCreditError> {
        require_monday(monday)?;
        if expected_revisions.week_start_date != monday {
            return Err(WeeklyCreditError::VectorOfAnotherWeek(
                expected_revisions.week_start_date,
            ));
        }
        let days = weekdays(monday);
        let mut keys = HashSet::new();
        for revision in revisions_to_append {
            let key = (revision.workday_date(), revision.revision());
            if !days.contains(&key.0) {
                return Err(WeeklyCreditError::RevisionOutsideWeek {
                    workday_date: key.0,
                    revision: key.1,
                });
            }
            if !keys.insert(key) {
                return Err(WeeklyCreditError::DuplicateRevision {
                    workday_date: key.0,
                    revision: key.1,
                });
            }
        }
        let records = replacement_records(monday, replacement_ledger)?;

        let transaction = crate::database::scope(self.connection, TransactionBehavior::Immediate)?;
        let actual = read_revision_vector(&transaction, monday)?;
        if actual != *expected_revisions {
            return Err(WeeklyCreditError::Stale {
                expected: *expected_revisions,
                actual,
            });
        }
        let mut ordered: Vec<_> = revisions_to_append.iter().collect();
        ordered.sort_by_key(|revision| (revision.workday_date(), revision.revision()));
        for revision in ordered {
            DailySummaryRepository::append_within(&transaction, revision)?;
        }
        let heads = read_heads(&transaction, monday)?;
        validate_consistency(monday, &heads, &records).map_err(WeeklyCreditError::Inconsistent)?;
        replace_week(&transaction, monday, &records)?;
        transaction.commit()?;
        Ok(())
    }
}

fn read_revision_vector(
    connection: &Connection,
    monday: Date,
) -> Result<WeeklyRevisionVector, WeeklyCreditError> {
    let days = weekdays(monday);
    let mut statement = connection.prepare(
        "SELECT WorkdayDate, MAX(Revision) AS Revision
FROM DailySummaryRevision
WHERE WorkdayDate BETWEEN $monday AND $friday
GROUP BY WorkdayDate;",
    )?;
    let rows = statement.query_map(
        named_params! {
            "$monday": format_date(days[0]),
            "$friday": format_date(days[4]),
        },
        |row| {
            let row = StoredRow::new(row, history::TABLE);
            Ok(row.date("WorkdayDate").and_then(|date| {
                let offset = days.iter().position(|day| *day == date).ok_or_else(|| {
                    row.invalid(format!(
                        "Stored daily summary date {} is outside its queried week.",
                        format_date(date)
                    ))
                })?;
                Ok((offset, row.integer("Revision")?))
            }))
        },
    )?;
    let mut revisions = [0; 5];
    for (offset, revision) in collect::<_, WeeklyCreditError>(rows)? {
        revisions[offset] = revision;
    }
    Ok(WeeklyRevisionVector {
        week_start_date: monday,
        revisions,
    })
}

/// The carry columns of the latest summary of each weekday, without decoding the rest.
fn read_heads(
    connection: &Connection,
    monday: Date,
) -> Result<BTreeMap<Date, Head>, WeeklyCreditError> {
    let [monday, .., friday] = weekdays(monday);
    let mut statement = connection.prepare(
        "SELECT
    current.WorkdayDate,
    current.Revision,
    current.CarryAvailableTicks,
    current.CarryEarnedTicks,
    current.CarryAppliedTicks,
    current.CarryExpiredTicks
FROM DailySummaryRevision AS current
WHERE current.WorkdayDate BETWEEN $monday AND $friday
    AND current.Revision = (
        SELECT MAX(candidate.Revision)
        FROM DailySummaryRevision AS candidate
        WHERE candidate.WorkdayDate = current.WorkdayDate);",
    )?;
    let rows = statement.query_map(
        named_params! {
            "$monday": format_date(monday),
            "$friday": format_date(friday),
        },
        |row| {
            let row = StoredRow::new(row, history::TABLE);
            let head = || -> Result<_, InvalidStoredValue> {
                Ok((
                    row.date("WorkdayDate")?,
                    Head {
                        revision: row.integer("Revision")?,
                        carry_available_ticks: row.integer("CarryAvailableTicks")?,
                        carry_earned_ticks: row.integer("CarryEarnedTicks")?,
                        carry_applied_ticks: row.integer("CarryAppliedTicks")?,
                        carry_expired_ticks: row.integer("CarryExpiredTicks")?,
                    },
                ))
            };
            Ok(head())
        },
    )?;
    Ok(collect::<_, WeeklyCreditError>(rows)?.into_iter().collect())
}

fn checked_sum(ticks: impl IntoIterator<Item = i64>) -> Result<i64, WeeklyInconsistency> {
    ticks.into_iter().try_fold(0i64, |sum, ticks| {
        sum.checked_add(ticks)
            .ok_or(WeeklyInconsistency::TotalsOverflow)
    })
}

fn validate_consistency(
    monday: Date,
    heads: &BTreeMap<Date, Head>,
    ledger: &[LedgerRecord],
) -> Result<(), WeeklyInconsistency> {
    if ledger
        .iter()
        .any(|entry| entry.kind() == CreditLedgerEntryKind::Reversed)
    {
        return Err(WeeklyInconsistency::ReversedMovement);
    }
    let friday = weekdays(monday)[4];
    for entry in ledger {
        let id = || entry.id().clone();
        let origin = entry.origin();
        if !heads.contains_key(&entry.workday_date()) {
            return Err(WeeklyInconsistency::DayWithoutSummary(id()));
        }
        if heads
            .get(&origin.workday_date)
            .is_none_or(|head| head.revision != origin.revision)
        {
            return Err(WeeklyInconsistency::UnselectedOrigin(id()));
        }
        match entry.kind() {
            CreditLedgerEntryKind::Earned if origin.workday_date != entry.workday_date() => {
                return Err(WeeklyInconsistency::EarnedFromAnotherDay(id()));
            }
            CreditLedgerEntryKind::Applied if origin.workday_date >= entry.workday_date() => {
                return Err(WeeklyInconsistency::AppliedFromLaterOrigin(id()));
            }
            CreditLedgerEntryKind::Expired if entry.workday_date() != friday => {
                return Err(WeeklyInconsistency::ExpiryNotOnFriday);
            }
            _ => {}
        }
    }

    for (&workday_date, head) in heads {
        for kind in [
            CreditLedgerEntryKind::Earned,
            CreditLedgerEntryKind::Applied,
            CreditLedgerEntryKind::Expired,
        ] {
            let ledger_ticks = checked_sum(
                ledger
                    .iter()
                    .filter(|entry| entry.workday_date() == workday_date && entry.kind() == kind)
                    .map(LedgerRecord::duration_ticks),
            )?;
            if ledger_ticks != head.total(kind) {
                return Err(WeeklyInconsistency::TotalMismatch {
                    workday_date,
                    kind,
                    ledger_ticks,
                    summary_ticks: head.total(kind),
                });
            }
        }
    }

    validate_lots(monday, heads, ledger)
}

/// An earned lot and what is left of it.
struct Lot {
    origin: CreditOrigin,
    remaining_ticks: i64,
}

/// Replays the week day by day: applied movements consume the oldest lots first, the day's
/// earned lots join, and expiry consumes what is left. Each day's remaining credit must equal its
/// summary's available carry.
fn validate_lots(
    monday: Date,
    heads: &BTreeMap<Date, Head>,
    ledger: &[LedgerRecord],
) -> Result<(), WeeklyInconsistency> {
    let mut lots: Vec<Lot> = Vec::new();
    for workday_date in weekdays(monday) {
        let by_origin = |kind| {
            let mut totals: BTreeMap<(Date, i64), i64> = BTreeMap::new();
            for entry in ledger
                .iter()
                .filter(|entry| entry.workday_date() == workday_date && entry.kind() == kind)
            {
                let origin = entry.origin();
                let total = totals
                    .entry((origin.workday_date, origin.revision))
                    .or_default();
                *total = total
                    .checked_add(entry.duration_ticks())
                    .ok_or(WeeklyInconsistency::TotalsOverflow)?;
            }
            Ok::<_, WeeklyInconsistency>(totals)
        };

        let applied = by_origin(CreditLedgerEntryKind::Applied)?;
        consume_fifo(
            workday_date,
            CreditLedgerEntryKind::Applied,
            &applied,
            &mut lots,
        )?;
        for ((origin_date, origin_revision), ticks) in by_origin(CreditLedgerEntryKind::Earned)? {
            lots.push(Lot {
                origin: CreditOrigin {
                    workday_date: origin_date,
                    revision: origin_revision,
                },
                remaining_ticks: ticks,
            });
        }
        let expired = by_origin(CreditLedgerEntryKind::Expired)?;
        consume_fifo(
            workday_date,
            CreditLedgerEntryKind::Expired,
            &expired,
            &mut lots,
        )?;

        let available = checked_sum(lots.iter().map(|lot| lot.remaining_ticks))?;
        if !expired.is_empty() && available != 0 {
            return Err(WeeklyInconsistency::PartialExpiry);
        }
        if let Some(head) = heads.get(&workday_date)
            && head.carry_available_ticks != available
        {
            return Err(WeeklyInconsistency::AvailableMismatch {
                workday_date,
                ledger_ticks: available,
                summary_ticks: head.carry_available_ticks,
            });
        }
    }
    Ok(())
}

/// Consumes `supplied`, the ticks drawn on each origin, from the oldest lots first, and checks
/// that the movements drew on exactly the lots FIFO picks, by the same amounts.
fn consume_fifo(
    workday_date: Date,
    kind: CreditLedgerEntryKind,
    supplied: &BTreeMap<(Date, i64), i64>,
    lots: &mut [Lot],
) -> Result<(), WeeklyInconsistency> {
    let mut remaining = checked_sum(supplied.values().copied())?;
    let mut expected = BTreeMap::new();
    for lot in lots.iter_mut().filter(|lot| lot.remaining_ticks > 0) {
        if remaining == 0 {
            break;
        }
        let consumed = lot.remaining_ticks.min(remaining);
        expected.insert((lot.origin.workday_date, lot.origin.revision), consumed);
        lot.remaining_ticks -= consumed;
        remaining -= consumed;
    }
    if remaining != 0 {
        return Err(WeeklyInconsistency::ExceedsEarnedLots { workday_date, kind });
    }
    if expected != *supplied {
        return Err(WeeklyInconsistency::NotFifo { workday_date, kind });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
