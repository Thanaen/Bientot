//! The daily summary history, ported from `History/DailySummaryRepository.cs`: each workday keeps
//! only its latest revision, and revisions advance one at a time.

mod breakdown;

#[cfg(test)]
pub(crate) use breakdown::serialize as serialize_breakdown;

use jiff::civil::Date;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, named_params};
use worktime_domain::history::{
    DailySummaryRevision, DailySummarySnapshot, DailySummarySnapshotParts,
};
use worktime_domain::sources::SourceQuality;
use worktime_domain::summaries::IndicatorState;
use worktime_domain::time::LocalWorkday;

use crate::codec::{
    DurationOutOfRange, format_date, format_instant, from_ticks, parse_instant, to_ticks,
};
use crate::row::{InvalidStoredValue, StorageError, StoredRow, collect, storage_error_conversions};

pub(crate) const TABLE: &str = "DailySummaryRevision";
const COLUMNS: &str = "WorkdayDate, Revision, ComputedAtUtc, TimezoneId, InputFingerprint, \
    CreditedTicks, ExcludedTicks, UnknownTicks, NominalTargetTicks, EffectiveTargetTicks, \
    CarryAvailableTicks, CarryAppliedTicks, CarryEarnedTicks, CarryExpiredTicks, \
    ProjectedDepartureUtc, IndicatorState, SourceQuality, HasPendingReview, BreakdownJson, \
    CarryDebtTicks, DebtPending";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppendDailySummaryOutcome {
    Inserted,
    AlreadyRecorded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppendDailySummaryResult {
    pub outcome: AppendDailySummaryOutcome,
    pub authoritative_revision: i64,
}

#[derive(Debug)]
pub enum DailySummaryError {
    Storage(StorageError),
    /// The revision is already stored with different content.
    RevisionConflict {
        workday_date: Date,
        revision: i64,
    },
    /// The revision does not follow the latest stored revision of its workday.
    RevisionSequence {
        workday_date: Date,
        expected_revision: i64,
        attempted_revision: i64,
    },
    DurationOutOfRange(DurationOutOfRange),
    /// Once truncated to the 100 ns ticks the database keeps, the revision is no longer a valid
    /// summary, so it could not be read back.
    NotStorable(InvalidStoredValue),
}

impl std::fmt::Display for DailySummaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(f),
            Self::RevisionConflict {
                workday_date,
                revision,
            } => write!(
                f,
                "Daily summary revision {revision} for {} already has different content.",
                format_date(*workday_date)
            ),
            Self::RevisionSequence {
                workday_date,
                expected_revision,
                attempted_revision,
            } => write!(
                f,
                "Daily summary revision {attempted_revision} for {} is out of sequence; expected \
                 revision {expected_revision}.",
                format_date(*workday_date)
            ),
            Self::DurationOutOfRange(error) => error.fmt(f),
            Self::NotStorable(error) => write!(
                f,
                "The daily summary is not valid at the 100 ns precision the database keeps: \
                 {error}"
            ),
        }
    }
}

impl std::error::Error for DailySummaryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::DurationOutOfRange(error) => Some(error),
            Self::NotStorable(error) => Some(error),
            _ => None,
        }
    }
}

storage_error_conversions!(DailySummaryError);

impl From<DurationOutOfRange> for DailySummaryError {
    fn from(error: DurationOutOfRange) -> Self {
        Self::DurationOutOfRange(error)
    }
}

const INDICATOR_STATES: [(IndicatorState, &str); 4] = [
    (IndicatorState::Green, "green"),
    (IndicatorState::Orange, "orange"),
    (IndicatorState::Red, "red"),
    (IndicatorState::Blue, "blue"),
];

const SOURCE_QUALITIES: [(SourceQuality, &str); 6] = [
    (SourceQuality::Complete, "complete"),
    (SourceQuality::Incomplete, "incomplete"),
    (SourceQuality::Ambiguous, "ambiguous"),
    (SourceQuality::Unauthorized, "unauthorized"),
    (SourceQuality::Incompatible, "incompatible"),
    (SourceQuality::Malformed, "malformed"),
];

/// The stored text of `value` in `table`, which names every variant.
fn stored_name<T: PartialEq + Copy>(table: &[(T, &'static str)], value: T) -> &'static str {
    table
        .iter()
        .find(|(candidate, _)| *candidate == value)
        .map(|(_, text)| *text)
        .expect("every variant has a stored name")
}

/// The variant `table` stores as `text`.
fn named<T: Copy>(table: &[(T, &'static str)], text: &str) -> Option<T> {
    table
        .iter()
        .find(|(_, candidate)| *candidate == text)
        .map(|(value, _)| *value)
}

fn invalid(detail: impl Into<String>) -> InvalidStoredValue {
    InvalidStoredValue {
        table: TABLE,
        detail: detail.into(),
    }
}

/// The columns of one revision, as stored. An append is a retry exactly when its record equals
/// the stored one, `BreakdownJson` text included.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SummaryRecord {
    workday_date: String,
    revision: i64,
    computed_at_utc: String,
    timezone_id: String,
    input_fingerprint: String,
    credited_ticks: i64,
    excluded_ticks: i64,
    unknown_ticks: i64,
    nominal_target_ticks: i64,
    effective_target_ticks: i64,
    carry_available_ticks: i64,
    carry_applied_ticks: i64,
    carry_earned_ticks: i64,
    carry_expired_ticks: i64,
    projected_departure_utc: Option<String>,
    indicator_state: String,
    source_quality: String,
    has_pending_review: i64,
    breakdown_json: String,
    carry_debt_ticks: i64,
    debt_pending: i64,
}

impl SummaryRecord {
    fn of(revision: &DailySummaryRevision) -> Result<Self, DurationOutOfRange> {
        let snapshot = revision.snapshot();
        Ok(Self {
            workday_date: format_date(snapshot.workday().date),
            revision: revision.revision(),
            computed_at_utc: format_instant(snapshot.computed_at()),
            timezone_id: snapshot.workday().time_zone_id.clone(),
            input_fingerprint: snapshot.input_fingerprint().to_owned(),
            credited_ticks: to_ticks(snapshot.credited_work())?,
            excluded_ticks: to_ticks(snapshot.excluded_work())?,
            unknown_ticks: to_ticks(snapshot.unknown_work())?,
            nominal_target_ticks: to_ticks(snapshot.nominal_target())?,
            effective_target_ticks: to_ticks(snapshot.effective_target())?,
            carry_available_ticks: to_ticks(snapshot.carry_available())?,
            carry_applied_ticks: to_ticks(snapshot.carry_applied())?,
            carry_earned_ticks: to_ticks(snapshot.carry_earned())?,
            carry_expired_ticks: to_ticks(snapshot.carry_expired())?,
            projected_departure_utc: snapshot.projected_departure().map(format_instant),
            indicator_state: stored_name(&INDICATOR_STATES, snapshot.indicator_state()).to_owned(),
            source_quality: stored_name(&SOURCE_QUALITIES, snapshot.source_quality()).to_owned(),
            has_pending_review: i64::from(snapshot.has_pending_review()),
            breakdown_json: breakdown::serialize(snapshot.breakdown())?,
            carry_debt_ticks: to_ticks(snapshot.carry_debt())?,
            debt_pending: i64::from(snapshot.debt_pending()),
        })
    }

    fn read(row: &StoredRow) -> Result<Self, InvalidStoredValue> {
        Ok(Self {
            workday_date: row.text("WorkdayDate")?.to_owned(),
            revision: row.integer("Revision")?,
            computed_at_utc: row.text("ComputedAtUtc")?.to_owned(),
            timezone_id: row.text("TimezoneId")?.to_owned(),
            input_fingerprint: row.text("InputFingerprint")?.to_owned(),
            credited_ticks: row.integer("CreditedTicks")?,
            excluded_ticks: row.integer("ExcludedTicks")?,
            unknown_ticks: row.integer("UnknownTicks")?,
            nominal_target_ticks: row.integer("NominalTargetTicks")?,
            effective_target_ticks: row.integer("EffectiveTargetTicks")?,
            carry_available_ticks: row.integer("CarryAvailableTicks")?,
            carry_applied_ticks: row.integer("CarryAppliedTicks")?,
            carry_earned_ticks: row.integer("CarryEarnedTicks")?,
            carry_expired_ticks: row.integer("CarryExpiredTicks")?,
            projected_departure_utc: row
                .optional_text("ProjectedDepartureUtc")?
                .map(str::to_owned),
            indicator_state: row.text("IndicatorState")?.to_owned(),
            source_quality: row.text("SourceQuality")?.to_owned(),
            has_pending_review: row.integer("HasPendingReview")?,
            breakdown_json: row.text("BreakdownJson")?.to_owned(),
            carry_debt_ticks: row.integer("CarryDebtTicks")?,
            debt_pending: row.integer("DebtPending")?,
        })
    }

    fn to_domain(&self) -> Result<DailySummaryRevision, InvalidStoredValue> {
        let flag = |value: i64, column: &str| match value {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid(format!("Unknown {column} value '{value}'."))),
        };
        let instant = |text: &str, column: &str| {
            parse_instant(text).map_err(|_| {
                invalid(format!(
                    "Stored {column} is not a canonical UTC round-trip timestamp."
                ))
            })
        };
        let snapshot = DailySummarySnapshot::new(DailySummarySnapshotParts {
            workday: LocalWorkday::new(
                crate::codec::parse_date(&self.workday_date)
                    .map_err(|error| invalid(format!("WorkdayDate: {error}")))?,
                self.timezone_id.clone(),
            ),
            computed_at: instant(&self.computed_at_utc, "ComputedAtUtc")?,
            input_fingerprint: self.input_fingerprint.clone(),
            credited_work: from_ticks(self.credited_ticks),
            excluded_work: from_ticks(self.excluded_ticks),
            unknown_work: from_ticks(self.unknown_ticks),
            nominal_target: from_ticks(self.nominal_target_ticks),
            effective_target: from_ticks(self.effective_target_ticks),
            carry_available: from_ticks(self.carry_available_ticks),
            carry_applied: from_ticks(self.carry_applied_ticks),
            carry_earned: from_ticks(self.carry_earned_ticks),
            carry_expired: from_ticks(self.carry_expired_ticks),
            carry_debt: from_ticks(self.carry_debt_ticks),
            debt_pending: flag(self.debt_pending, "pending-debt")?,
            projected_departure: self
                .projected_departure_utc
                .as_deref()
                .map(|text| instant(text, "ProjectedDepartureUtc"))
                .transpose()?,
            indicator_state: named(&INDICATOR_STATES, &self.indicator_state).ok_or_else(|| {
                invalid(format!(
                    "Unknown indicator state '{}'.",
                    self.indicator_state
                ))
            })?,
            source_quality: named(&SOURCE_QUALITIES, &self.source_quality).ok_or_else(|| {
                invalid(format!("Unknown source quality '{}'.", self.source_quality))
            })?,
            has_pending_review: flag(self.has_pending_review, "pending-review")?,
            breakdown: breakdown::deserialize(&self.breakdown_json).map_err(invalid)?,
        })
        .map_err(|error| invalid(error.to_string()))?;
        DailySummaryRevision::new(self.revision, snapshot)
            .map_err(|error| invalid(error.to_string()))
    }
}

fn parse(row: &StoredRow) -> Result<DailySummaryRevision, InvalidStoredValue> {
    SummaryRecord::read(row)?.to_domain()
}

pub struct DailySummaryRepository<'a> {
    connection: &'a Connection,
}

impl<'a> DailySummaryRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Appends `revision` as the latest of its workday and drops the revisions it supersedes.
    /// Appending the stored latest revision again is a retry and changes nothing.
    pub fn append(
        &self,
        revision: &DailySummaryRevision,
    ) -> Result<AppendDailySummaryResult, DailySummaryError> {
        let transaction = crate::database::scope(self.connection, TransactionBehavior::Immediate)?;
        let result = Self::append_within(&transaction, revision)?;
        transaction.commit()?;
        Ok(result)
    }

    /// [`Self::append`] inside a transaction the caller commits or rolls back.
    pub fn append_within(
        transaction: &Connection,
        revision: &DailySummaryRevision,
    ) -> Result<AppendDailySummaryResult, DailySummaryError> {
        let expected = record(revision)?;
        let workday_date = revision.workday_date();
        if let Some(existing) = find(transaction, &expected.workday_date, expected.revision)? {
            if existing != expected {
                return Err(DailySummaryError::RevisionConflict {
                    workday_date,
                    revision: expected.revision,
                });
            }
            return Ok(AppendDailySummaryResult {
                outcome: AppendDailySummaryOutcome::AlreadyRecorded,
                authoritative_revision: expected.revision,
            });
        }

        let latest: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(Revision), 0) FROM DailySummaryRevision \
             WHERE WorkdayDate = $workdayDate;",
            named_params! { "$workdayDate": expected.workday_date },
            |row| row.get(0),
        )?;
        // .NET throws an OverflowException past i64::MAX; no workday gets that many revisions.
        let next = latest.saturating_add(1);
        if expected.revision != next {
            return Err(DailySummaryError::RevisionSequence {
                workday_date,
                expected_revision: next,
                attempted_revision: expected.revision,
            });
        }

        insert(transaction, &expected)?;
        transaction.execute(
            "DELETE FROM DailySummaryRevision WHERE WorkdayDate = $workdayDate AND Revision < $revision;",
            named_params! {
                "$workdayDate": expected.workday_date,
                "$revision": expected.revision,
            },
        )?;
        Ok(AppendDailySummaryResult {
            outcome: AppendDailySummaryOutcome::Inserted,
            authoritative_revision: expected.revision,
        })
    }

    pub fn read_latest(
        &self,
        workday_date: Date,
    ) -> Result<Option<DailySummaryRevision>, DailySummaryError> {
        let found = self
            .connection
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM DailySummaryRevision \
                     WHERE WorkdayDate = $workdayDate ORDER BY Revision DESC LIMIT 1;"
                ),
                named_params! { "$workdayDate": format_date(workday_date) },
                |row| Ok(parse(&StoredRow::new(row, TABLE))),
            )
            .optional()?;
        Ok(found.transpose()?)
    }

    /// The latest revision of each weekday of the week containing `date`, by date.
    pub fn read_latest_week(
        &self,
        date: Date,
    ) -> Result<Vec<DailySummaryRevision>, DailySummaryError> {
        read_latest_week(self.connection, start_of_week(date))
    }
}

/// The Monday of the week containing `date`.
pub(crate) fn start_of_week(date: Date) -> Date {
    date.checked_sub(jiff::Span::new().days(date.weekday().to_monday_zero_offset()))
        .expect("every Monday is within the calendar")
}

/// Monday to Friday of the week starting on `monday`.
pub(crate) fn weekdays(monday: Date) -> [Date; 5] {
    std::array::from_fn(|offset| {
        monday
            .checked_add(jiff::Span::new().days(offset as i64))
            .expect("the calendar ends on a Friday")
    })
}

pub(crate) fn read_latest_week(
    connection: &Connection,
    monday: Date,
) -> Result<Vec<DailySummaryRevision>, DailySummaryError> {
    let [monday, .., friday] = weekdays(monday);
    let mut statement = connection.prepare(&format!(
        "SELECT {COLUMNS}
FROM DailySummaryRevision AS current
WHERE current.WorkdayDate BETWEEN $monday AND $friday
    AND current.Revision = (
        SELECT MAX(candidate.Revision)
        FROM DailySummaryRevision AS candidate
        WHERE candidate.WorkdayDate = current.WorkdayDate)
ORDER BY current.WorkdayDate;"
    ))?;
    let rows = statement.query_map(
        named_params! {
            "$monday": format_date(monday),
            "$friday": format_date(friday),
        },
        |row| Ok(parse(&StoredRow::new(row, TABLE))),
    )?;
    collect(rows)
}

fn find(
    connection: &Connection,
    workday_date: &str,
    revision: i64,
) -> Result<Option<SummaryRecord>, DailySummaryError> {
    let found = connection
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM DailySummaryRevision \
                 WHERE WorkdayDate = $workdayDate AND Revision = $revision;"
            ),
            named_params! { "$workdayDate": workday_date, "$revision": revision },
            |row| Ok(SummaryRecord::read(&StoredRow::new(row, TABLE))),
        )
        .optional()?;
    Ok(found.transpose()?)
}

fn insert(connection: &Connection, record: &SummaryRecord) -> Result<(), DailySummaryError> {
    connection.execute(
        "INSERT INTO DailySummaryRevision (
    WorkdayDate,
    Revision,
    ComputedAtUtc,
    TimezoneId,
    InputFingerprint,
    CreditedTicks,
    ExcludedTicks,
    UnknownTicks,
    NominalTargetTicks,
    EffectiveTargetTicks,
    CarryAvailableTicks,
    CarryAppliedTicks,
    CarryEarnedTicks,
    CarryExpiredTicks,
    ProjectedDepartureUtc,
    IndicatorState,
    SourceQuality,
    HasPendingReview,
    BreakdownJson,
    CarryDebtTicks,
    DebtPending)
VALUES (
    $workdayDate,
    $revision,
    $computedAtUtc,
    $timezoneId,
    $inputFingerprint,
    $creditedTicks,
    $excludedTicks,
    $unknownTicks,
    $nominalTargetTicks,
    $effectiveTargetTicks,
    $carryAvailableTicks,
    $carryAppliedTicks,
    $carryEarnedTicks,
    $carryExpiredTicks,
    $projectedDepartureUtc,
    $indicatorState,
    $sourceQuality,
    $hasPendingReview,
    $breakdownJson,
    $carryDebtTicks,
    $debtPending);",
        named_params! {
            "$workdayDate": record.workday_date,
            "$revision": record.revision,
            "$computedAtUtc": record.computed_at_utc,
            "$timezoneId": record.timezone_id,
            "$inputFingerprint": record.input_fingerprint,
            "$creditedTicks": record.credited_ticks,
            "$excludedTicks": record.excluded_ticks,
            "$unknownTicks": record.unknown_ticks,
            "$nominalTargetTicks": record.nominal_target_ticks,
            "$effectiveTargetTicks": record.effective_target_ticks,
            "$carryAvailableTicks": record.carry_available_ticks,
            "$carryAppliedTicks": record.carry_applied_ticks,
            "$carryEarnedTicks": record.carry_earned_ticks,
            "$carryExpiredTicks": record.carry_expired_ticks,
            "$projectedDepartureUtc": record.projected_departure_utc,
            "$indicatorState": record.indicator_state,
            "$sourceQuality": record.source_quality,
            "$hasPendingReview": record.has_pending_review,
            "$breakdownJson": record.breakdown_json,
            "$carryDebtTicks": record.carry_debt_ticks,
            "$debtPending": record.debt_pending,
        },
    )?;
    Ok(())
}

/// `revision` as the database reads it back: durations and instants truncated to ticks.
pub fn stored_form(
    revision: &DailySummaryRevision,
) -> Result<DailySummaryRevision, DailySummaryError> {
    record(revision)?
        .to_domain()
        .map_err(DailySummaryError::NotStorable)
}

fn record(revision: &DailySummaryRevision) -> Result<SummaryRecord, DailySummaryError> {
    // The record keeps durations and instants in ticks. What it holds must read back, and its
    // breakdown entries take the order they have at tick precision, as .NET orders them.
    let mut expected = SummaryRecord::of(revision)?;
    let stored = expected
        .to_domain()
        .map_err(DailySummaryError::NotStorable)?;
    expected.breakdown_json = breakdown::serialize(stored.snapshot().breakdown())?;
    Ok(expected)
}

#[cfg(test)]
mod tests;
