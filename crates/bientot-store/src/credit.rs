//! The weekly credit ledger, ported from `Credit/CreditLedgerRepository.cs`: each Monday-to-Friday
//! week is replaced as a whole.

mod weekly;

pub use weekly::{
    InvalidRevisionVector, WeeklyCreditError, WeeklyCreditRepository, WeeklyCreditState,
    WeeklyInconsistency, WeeklyRevisionVector,
};

use bientot_domain::credit::{
    CreditLedgerEntry, CreditLedgerEntryId, CreditLedgerEntryKind, CreditOrigin,
};
use jiff::civil::{Date, Weekday};
use rusqlite::{Connection, TransactionBehavior, named_params};

use crate::codec::{DurationOutOfRange, format_date, format_instant, from_ticks, to_ticks};
use crate::row::{InvalidStoredValue, StorageError, StoredRow, collect, storage_error_conversions};

const KINDS: [(CreditLedgerEntryKind, &str); 4] = [
    (CreditLedgerEntryKind::Earned, "earned"),
    (CreditLedgerEntryKind::Applied, "applied"),
    (CreditLedgerEntryKind::Expired, "expired"),
    (CreditLedgerEntryKind::Reversed, "reversed"),
];

fn kind_storage(kind: CreditLedgerEntryKind) -> &'static str {
    KINDS
        .iter()
        .find(|(candidate, _)| *candidate == kind)
        .map(|(_, text)| *text)
        .expect("every kind has a stored name")
}

#[derive(Debug)]
pub enum CreditLedgerError {
    Storage(StorageError),
    /// A ledger week starts on Monday.
    NotAMonday(Date),
    /// A replacement holds only entries of the week it replaces.
    EntryOutsideWeek(CreditLedgerEntryId),
    DurationOutOfRange(DurationOutOfRange),
}

impl std::fmt::Display for CreditLedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(f),
            Self::NotAMonday(date) => write!(
                f,
                "A credit ledger week must start on Monday, not {}.",
                format_date(*date)
            ),
            Self::EntryOutsideWeek(id) => write!(
                f,
                "Credit ledger entry {id} does not belong to the week being replaced."
            ),
            Self::DurationOutOfRange(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for CreditLedgerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::DurationOutOfRange(error) => Some(error),
            _ => None,
        }
    }
}

storage_error_conversions!(CreditLedgerError);

pub(crate) fn require_monday(date: Date) -> Result<(), CreditLedgerError> {
    if date.weekday() == Weekday::Monday {
        Ok(())
    } else {
        Err(CreditLedgerError::NotAMonday(date))
    }
}

/// The columns of one entry, as stored: instants and amounts in .NET ticks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LedgerRecord {
    id: CreditLedgerEntryId,
    workday_date: Date,
    week_start_date: Date,
    kind: CreditLedgerEntryKind,
    duration_ticks: i64,
    origin: CreditOrigin,
    created_at_utc: String,
}

impl LedgerRecord {
    pub(crate) fn id(&self) -> &CreditLedgerEntryId {
        &self.id
    }

    pub(crate) fn workday_date(&self) -> Date {
        self.workday_date
    }

    pub(crate) fn kind(&self) -> CreditLedgerEntryKind {
        self.kind
    }

    pub(crate) fn duration_ticks(&self) -> i64 {
        self.duration_ticks
    }

    pub(crate) fn origin(&self) -> CreditOrigin {
        self.origin
    }
}

/// The records of a replacement of the week starting on `monday`, checked as .NET checks them
/// before it opens the database.
pub(crate) fn replacement_records(
    monday: Date,
    entries: &[CreditLedgerEntry],
) -> Result<Vec<LedgerRecord>, CreditLedgerError> {
    require_monday(monday)?;
    if let Some(outside) = entries
        .iter()
        .find(|entry| entry.week_start_date() != monday)
    {
        return Err(CreditLedgerError::EntryOutsideWeek(outside.id().clone()));
    }
    entries
        .iter()
        .map(|entry| {
            Ok(LedgerRecord {
                id: entry.id().clone(),
                workday_date: entry.workday_date(),
                week_start_date: entry.week_start_date(),
                kind: entry.kind(),
                duration_ticks: to_ticks(entry.amount())
                    .map_err(CreditLedgerError::DurationOutOfRange)?,
                origin: entry.origin(),
                created_at_utc: format_instant(entry.created_at()),
            })
        })
        .collect()
}

pub struct CreditLedgerRepository<'a> {
    connection: &'a Connection,
}

impl<'a> CreditLedgerRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// The entries of the week starting on `monday`, by workday, creation instant and id.
    pub fn read_week(&self, monday: Date) -> Result<Vec<CreditLedgerEntry>, CreditLedgerError> {
        require_monday(monday)?;
        read_week(self.connection, monday)
    }

    /// Replaces every entry of the week starting on `monday` with `entries`, at once. No entry
    /// clears the week.
    pub fn replace_week(
        &self,
        monday: Date,
        entries: &[CreditLedgerEntry],
    ) -> Result<(), CreditLedgerError> {
        let records = replacement_records(monday, entries)?;
        let transaction = crate::database::scope(self.connection, TransactionBehavior::Immediate)?;
        replace_week(&transaction, monday, &records)?;
        transaction.commit()?;
        Ok(())
    }
}

pub(crate) fn read_week(
    connection: &Connection,
    monday: Date,
) -> Result<Vec<CreditLedgerEntry>, CreditLedgerError> {
    let mut statement = connection.prepare(
        "SELECT
    Id,
    WorkdayDate,
    Kind,
    DurationTicks,
    OriginWorkdayDate,
    OriginRevision,
    CreatedAtUtc
FROM CreditLedgerEntry
WHERE WeekStartDate = $weekStartDate
ORDER BY WorkdayDate, CreatedAtUtc, Id;",
    )?;
    let rows = statement.query_map(
        named_params! { "$weekStartDate": format_date(monday) },
        |row| Ok(parse(&StoredRow::new(row, "CreditLedgerEntry"))),
    )?;
    collect(rows)
}

pub(crate) fn replace_week(
    transaction: &Connection,
    monday: Date,
    records: &[LedgerRecord],
) -> Result<(), CreditLedgerError> {
    transaction.execute(
        "DELETE FROM CreditLedgerEntry WHERE WeekStartDate = $weekStartDate;",
        named_params! { "$weekStartDate": format_date(monday) },
    )?;
    for record in records {
        transaction.execute(
            "INSERT INTO CreditLedgerEntry (
    Id,
    WorkdayDate,
    WeekStartDate,
    Kind,
    DurationTicks,
    OriginWorkdayDate,
    OriginRevision,
    CreatedAtUtc)
VALUES (
    $id,
    $workdayDate,
    $weekStartDate,
    $kind,
    $durationTicks,
    $originWorkdayDate,
    $originRevision,
    $createdAtUtc);",
            named_params! {
                "$id": record.id.as_str(),
                "$workdayDate": format_date(record.workday_date),
                "$weekStartDate": format_date(record.week_start_date),
                "$kind": kind_storage(record.kind),
                "$durationTicks": record.duration_ticks,
                "$originWorkdayDate": format_date(record.origin.workday_date),
                "$originRevision": record.origin.revision,
                "$createdAtUtc": record.created_at_utc,
            },
        )?;
    }
    Ok(())
}

fn parse(row: &StoredRow) -> Result<CreditLedgerEntry, InvalidStoredValue> {
    let id = CreditLedgerEntryId::new(row.text("Id")?).map_err(|_| row.invalid("Id is blank"))?;
    let workday_date = row.date("WorkdayDate")?;
    let kind = row.text("Kind")?;
    let kind = KINDS
        .iter()
        .find(|(_, candidate)| *candidate == kind)
        .map(|(kind, _)| *kind)
        .ok_or_else(|| row.invalid(format!("Unknown credit ledger kind '{kind}'.")))?;
    let amount = from_ticks(row.integer("DurationTicks")?);
    let origin = CreditOrigin {
        workday_date: row.date("OriginWorkdayDate")?,
        revision: row.integer("OriginRevision")?,
    };
    let created_at = row.lenient_instant("CreatedAtUtc")?;
    CreditLedgerEntry::new(id, workday_date, kind, amount, origin, created_at)
        .map_err(|error| row.invalid(error.to_string()))
}

#[cfg(test)]
mod tests;
