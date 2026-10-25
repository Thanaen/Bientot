//! The append-only history of manual adjustments, ported from
//! `Adjustments/ManualAdjustmentRepository.cs`.

use bientot_domain::accounting::{AccountingResult, AdjustmentId, LongLockReviewId};
use bientot_domain::adjustments::{
    AdjustmentKind, AdjustmentValidationError, ManualAdjustment, ManualClassification,
    validate_for_append,
};
use bientot_domain::intervals::UtcInterval;
use bientot_domain::time::LocalWorkday;
use jiff::{SignedDuration, Timestamp};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, named_params};

use crate::codec::{format_date, format_instant, to_ticks, truncate_to_tick};
use crate::reviews::{self, LongLockResolutionClaim};
use crate::row::{InvalidStoredValue, StorageError, StoredRow, collect, storage_error_conversions};

const COLUMNS: &str = "Id, WorkdayDate, Kind, StartUtc, EndUtc, DurationMinutes, FromLabel, \
    ToLabel, Reason, CreatedAtUtc, SupersedesId, TimezoneId, ConfirmsReviewId";
const TICKS_PER_MINUTE: i64 = 600_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppendAdjustmentOutcome {
    Inserted,
    AlreadyRecorded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendAdjustmentResult {
    pub outcome: AppendAdjustmentOutcome,
    pub authoritative_id: AdjustmentId,
}

/// Why the adjustment a successor names cannot be superseded by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParentConflictReason {
    NotFound,
    DifferentWorkdayDate,
    DifferentTimezone,
    NonIncreasingCreatedAt,
}

#[derive(Debug)]
pub enum ManualAdjustmentError {
    Storage(StorageError),
    /// The id already identifies a different adjustment.
    IdConflict(AdjustmentId),
    ParentConflict {
        superseded_id: AdjustmentId,
        reason: ParentConflictReason,
    },
    /// The superseded adjustment already has a successor.
    SupersessionConflict {
        superseded_id: AdjustmentId,
        existing_successor_id: AdjustmentId,
    },
    /// The candidate would make the history of its workday invalid.
    Validation(AdjustmentValidationError),
    /// `DurationMinutes` holds whole minutes only.
    FractionalMinutes(AdjustmentId),
    /// The interval is gone once its ends are truncated to the 100 ns the database keeps.
    IntervalShorterThanATick(AdjustmentId),
    NotAWorkingDay,
    BlankTimeZone,
    /// The claim the resolution was recorded under was released before it could be recorded.
    ClaimReleased(AdjustmentId),
}

impl std::fmt::Display for ManualAdjustmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(f),
            Self::IdConflict(id) => write!(
                f,
                "Adjustment ID '{id}' already identifies a different adjustment."
            ),
            Self::ParentConflict {
                superseded_id,
                reason,
            } => match reason {
                ParentConflictReason::NotFound => {
                    write!(f, "Superseded adjustment '{superseded_id}' does not exist.")
                }
                ParentConflictReason::DifferentWorkdayDate => write!(
                    f,
                    "Superseded adjustment '{superseded_id}' belongs to a different workday date."
                ),
                ParentConflictReason::DifferentTimezone => write!(
                    f,
                    "Superseded adjustment '{superseded_id}' belongs to a different timezone."
                ),
                ParentConflictReason::NonIncreasingCreatedAt => write!(
                    f,
                    "A successor of adjustment '{superseded_id}' must have a later creation \
                     instant."
                ),
            },
            Self::SupersessionConflict {
                superseded_id,
                existing_successor_id,
            } => write!(
                f,
                "Adjustment '{superseded_id}' is already superseded by '{existing_successor_id}'."
            ),
            Self::Validation(error) => error.fmt(f),
            Self::FractionalMinutes(id) => write!(
                f,
                "Duration adjustment '{id}' must use whole minutes for persistence."
            ),
            Self::IntervalShorterThanATick(id) => write!(
                f,
                "Adjustment '{id}' covers less than the 100 ns the database keeps."
            ),
            Self::NotAWorkingDay => f.write_str("Adjustments belong to Monday-Friday workdays."),
            Self::BlankTimeZone => f.write_str("The workday time zone cannot be blank."),
            Self::ClaimReleased(id) => write!(
                f,
                "The review claim of resolution '{id}' was released before it was recorded."
            ),
        }
    }
}

impl std::error::Error for ManualAdjustmentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::Validation(error) => Some(error),
            _ => None,
        }
    }
}

storage_error_conversions!(ManualAdjustmentError);

/// The columns of one adjustment, as stored. Two adjustments are equal exactly when their records
/// are, once the instants are truncated to .NET ticks.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AdjustmentRecord {
    id: String,
    workday_date: String,
    kind: &'static str,
    start_utc: Option<String>,
    end_utc: Option<String>,
    duration_minutes: Option<i64>,
    to_label: Option<&'static str>,
    reason: Option<String>,
    created_at_utc: String,
    supersedes_id: Option<String>,
    timezone_id: String,
    confirms_review_id: Option<String>,
}

impl AdjustmentRecord {
    fn of(adjustment: &ManualAdjustment) -> Result<Self, ManualAdjustmentError> {
        let interval = adjustment.interval();
        let (kind, duration_minutes, to_label, confirms_review_id) = match adjustment.kind() {
            AdjustmentKind::IntervalWork { .. } => ("add-work", None, None, None),
            AdjustmentKind::DurationWork { duration } => (
                "add-work",
                Some(whole_minutes(adjustment, *duration)?),
                None,
                None,
            ),
            AdjustmentKind::Exclusion { .. } => ("exclude-time", None, None, None),
            AdjustmentKind::Reclassification {
                classification,
                confirms_review_id,
                ..
            } => (
                "reclassify",
                None,
                Some(match classification {
                    ManualClassification::Credited => "credited",
                    ManualClassification::Excluded => "excluded",
                }),
                confirms_review_id.as_ref().map(|id| id.as_str().to_owned()),
            ),
            AdjustmentKind::Revocation => ("revoke", None, None, None),
        };
        Ok(Self {
            id: adjustment.id().as_str().to_owned(),
            workday_date: format_date(adjustment.workday().date),
            kind,
            start_utc: interval.map(|interval| format_instant(interval.start())),
            end_utc: interval.map(|interval| format_instant(interval.end())),
            duration_minutes,
            to_label,
            reason: adjustment.reason().map(str::to_owned),
            created_at_utc: format_instant(adjustment.created_at()),
            supersedes_id: adjustment.supersedes_id().map(|id| id.as_str().to_owned()),
            timezone_id: adjustment.workday().time_zone_id.clone(),
            confirms_review_id,
        })
    }
}

/// `adjustment` as the database stores it: instants truncated to .NET ticks. Validation, the
/// retry comparison and the row all see the same values.
fn at_tick_precision(
    adjustment: &ManualAdjustment,
) -> Result<ManualAdjustment, ManualAdjustmentError> {
    let interval = |interval: &UtcInterval| {
        UtcInterval::new(
            truncate_to_tick(interval.start()),
            truncate_to_tick(interval.end()),
        )
        .map_err(|_| ManualAdjustmentError::IntervalShorterThanATick(adjustment.id().clone()))
    };
    let kind = match adjustment.kind() {
        AdjustmentKind::IntervalWork { interval: work } => AdjustmentKind::IntervalWork {
            interval: interval(work)?,
        },
        AdjustmentKind::Exclusion { interval: excluded } => AdjustmentKind::Exclusion {
            interval: interval(excluded)?,
        },
        AdjustmentKind::Reclassification {
            interval: reclassified,
            classification,
            confirms_review_id,
        } => AdjustmentKind::Reclassification {
            interval: interval(reclassified)?,
            classification: *classification,
            confirms_review_id: confirms_review_id.clone(),
        },
        kind @ (AdjustmentKind::DurationWork { .. } | AdjustmentKind::Revocation) => kind.clone(),
    };
    let stored = ManualAdjustment::new(
        adjustment.id().clone(),
        adjustment.workday().clone(),
        truncate_to_tick(adjustment.created_at()),
        adjustment.supersedes_id().cloned(),
        kind,
    )
    .expect("truncating instants keeps a valid adjustment valid");
    Ok(match adjustment.reason() {
        Some(reason) => stored.with_reason(reason),
        None => stored,
    })
}

fn whole_minutes(
    adjustment: &ManualAdjustment,
    duration: SignedDuration,
) -> Result<i64, ManualAdjustmentError> {
    let fractional = || ManualAdjustmentError::FractionalMinutes(adjustment.id().clone());
    let ticks = to_ticks(duration).map_err(|_| fractional())?;
    if ticks % TICKS_PER_MINUTE != 0 {
        return Err(fractional());
    }
    Ok(ticks / TICKS_PER_MINUTE)
}

pub struct ManualAdjustmentRepository<'a> {
    connection: &'a Connection,
}

impl<'a> ManualAdjustmentRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// A transaction over the connection, in which every write of the repositories on it, and
    /// of what they call, commits or rolls back as one.
    pub fn scope(&self) -> Result<crate::database::Scope<'a>, ManualAdjustmentError> {
        Ok(crate::database::scope(
            self.connection,
            TransactionBehavior::Immediate,
        )?)
    }

    /// Appends `adjustment` unless the same adjustment is already stored.
    pub fn append(
        &self,
        adjustment: &ManualAdjustment,
    ) -> Result<AppendAdjustmentResult, ManualAdjustmentError> {
        self.append_in_transaction(None, None, adjustment)
    }

    /// Appends `candidate` if the history of its workday, read in the same transaction, stays
    /// valid for the workday that `current` accounts.
    pub fn append_validated(
        &self,
        current: &AccountingResult,
        candidate: &ManualAdjustment,
    ) -> Result<AppendAdjustmentResult, ManualAdjustmentError> {
        self.append_in_transaction(Some(current), None, candidate)
    }

    /// As [`Self::append_validated`], for a review resolution that is recorded only while
    /// `claim` still holds its review, as acquired at the candidate's creation instant.
    pub fn append_claimed(
        &self,
        current: &AccountingResult,
        candidate: &ManualAdjustment,
        claim: &LongLockResolutionClaim,
    ) -> Result<AppendAdjustmentResult, ManualAdjustmentError> {
        self.append_in_transaction(Some(current), Some(claim), candidate)
    }

    /// Deletes `claim` when it is held by exactly this action and `resolution`, the adjustment
    /// it protects, is not recorded. Callers repeating one action share its claim, so another one
    /// may already have recorded the resolution; the check and the delete are one transaction. An
    /// unrelated adjustment under the same id does not keep the claim, and only the claim acquired
    /// at the resolution's creation instant is deleted, never one the action acquired again
    /// since. Returns whether the claim was deleted.
    pub fn release_unrecorded(
        &self,
        claim: &LongLockResolutionClaim,
        resolution: &ManualAdjustment,
    ) -> Result<bool, ManualAdjustmentError> {
        let resolution = &at_tick_precision(resolution)?;
        let transaction = crate::database::scope(self.connection, TransactionBehavior::Immediate)?;
        let recorded = find_by_id(
            &transaction,
            resolution.id(),
            &resolution.workday().time_zone_id,
        )?
        .is_some_and(|existing| existing == *resolution);
        let deleted =
            !recorded && reviews::delete(&transaction, claim, Some(resolution.created_at()))?;
        transaction.commit()?;
        Ok(deleted)
    }

    /// When the latest adjustment of any workday was created, `None` before the first.
    pub fn latest_created_at(&self) -> Result<Option<Timestamp>, ManualAdjustmentError> {
        let found = self
            .connection
            .query_row(
                "SELECT CreatedAtUtc FROM ManualAdjustment ORDER BY CreatedAtUtc DESC LIMIT 1;",
                [],
                |row| Ok(StoredRow::new(row, "ManualAdjustment").instant("CreatedAtUtc")),
            )
            .optional()?;
        Ok(found.transpose()?)
    }

    /// The adjustments of `workday`, by creation instant then id. Rows written before the
    /// adjustments recorded a time zone belong to whichever zone is asked for.
    pub fn read_for_workday(
        &self,
        workday: &LocalWorkday,
    ) -> Result<Vec<ManualAdjustment>, ManualAdjustmentError> {
        if !workday.is_working_day() {
            return Err(ManualAdjustmentError::NotAWorkingDay);
        }
        if workday.time_zone_id.trim().is_empty() {
            return Err(ManualAdjustmentError::BlankTimeZone);
        }
        read_for_workday(self.connection, workday)
    }

    /// Appends every adjustment of `candidates`, in order, in one transaction: each one keeps
    /// the history of its workday, with the ones before it, valid for the workday that
    /// `current` accounts, or none is recorded. An adjustment already stored is skipped, so a
    /// retry of the same batch records it once.
    pub fn append_all_validated(
        &self,
        current: &AccountingResult,
        candidates: &[ManualAdjustment],
    ) -> Result<Vec<AppendAdjustmentResult>, ManualAdjustmentError> {
        let transaction = crate::database::scope(self.connection, TransactionBehavior::Immediate)?;
        let mut histories: Vec<(LocalWorkday, Vec<ManualAdjustment>)> = Vec::new();
        let mut results = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let workday = candidate.workday();
            let index = match histories.iter().position(|(known, _)| known == workday) {
                Some(index) => index,
                None => {
                    histories.push((workday.clone(), read_for_workday(&transaction, workday)?));
                    histories.len() - 1
                }
            };
            let history = &mut histories[index].1;
            let (result, appended) =
                append_within(&transaction, Some(current), None, candidate, history)?;
            history.extend(appended);
            results.push(result);
        }
        transaction.commit()?;
        Ok(results)
    }

    fn append_in_transaction(
        &self,
        current: Option<&AccountingResult>,
        claim: Option<&LongLockResolutionClaim>,
        adjustment: &ManualAdjustment,
    ) -> Result<AppendAdjustmentResult, ManualAdjustmentError> {
        let transaction = crate::database::scope(self.connection, TransactionBehavior::Immediate)?;
        let history = match current {
            Some(_) => read_for_workday(&transaction, adjustment.workday())?,
            None => Vec::new(),
        };
        let (result, _) = append_within(&transaction, current, claim, adjustment, &history)?;
        transaction.commit()?;
        Ok(result)
    }
}

/// Appends `adjustment` within `transaction`, validated against `history`, its workday's
/// adjustments, when `current` is given. Returns the adjustment it inserted, if any, as stored.
fn append_within(
    transaction: &Connection,
    current: Option<&AccountingResult>,
    claim: Option<&LongLockResolutionClaim>,
    adjustment: &ManualAdjustment,
    history: &[ManualAdjustment],
) -> Result<(AppendAdjustmentResult, Option<ManualAdjustment>), ManualAdjustmentError> {
    let adjustment = at_tick_precision(adjustment)?;
    let record = AdjustmentRecord::of(&adjustment)?;
    let time_zone_id = &adjustment.workday().time_zone_id;
    if let Some(current) = current {
        validate_for_append(current, history, &adjustment)
            .map_err(ManualAdjustmentError::Validation)?;
    }

    if let Some(existing) = find_by_id(transaction, adjustment.id(), time_zone_id)? {
        if AdjustmentRecord::of(&existing)? != record {
            return Err(ManualAdjustmentError::IdConflict(adjustment.id().clone()));
        }
        let result = AppendAdjustmentResult {
            outcome: AppendAdjustmentOutcome::AlreadyRecorded,
            authoritative_id: existing.id().clone(),
        };
        return Ok((result, None));
    }

    if let Some(superseded_id) = adjustment.supersedes_id() {
        let parent = find_by_id(transaction, superseded_id, time_zone_id)?;
        validate_parent(&adjustment, superseded_id, parent.as_ref())?;
        if let Some(existing_successor_id) = find_successor_id(transaction, superseded_id)? {
            return Err(ManualAdjustmentError::SupersessionConflict {
                superseded_id: superseded_id.clone(),
                existing_successor_id,
            });
        }
    }

    if let Some(claim) = claim
        && !reviews::is_held(transaction, claim, adjustment.created_at())?
    {
        return Err(ManualAdjustmentError::ClaimReleased(
            adjustment.id().clone(),
        ));
    }
    insert(transaction, &record)?;
    let result = AppendAdjustmentResult {
        outcome: AppendAdjustmentOutcome::Inserted,
        authoritative_id: adjustment.id().clone(),
    };
    Ok((result, Some(adjustment)))
}

fn validate_parent(
    child: &ManualAdjustment,
    superseded_id: &AdjustmentId,
    parent: Option<&ManualAdjustment>,
) -> Result<(), ManualAdjustmentError> {
    let reason = match parent {
        None => ParentConflictReason::NotFound,
        Some(parent) if parent.workday().date != child.workday().date => {
            ParentConflictReason::DifferentWorkdayDate
        }
        Some(parent) if parent.workday().time_zone_id != child.workday().time_zone_id => {
            ParentConflictReason::DifferentTimezone
        }
        Some(parent) if child.created_at() <= parent.created_at() => {
            ParentConflictReason::NonIncreasingCreatedAt
        }
        Some(_) => return Ok(()),
    };
    Err(ManualAdjustmentError::ParentConflict {
        superseded_id: superseded_id.clone(),
        reason,
    })
}

fn read_for_workday(
    connection: &Connection,
    workday: &LocalWorkday,
) -> Result<Vec<ManualAdjustment>, ManualAdjustmentError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {COLUMNS}
FROM ManualAdjustment
WHERE WorkdayDate = $workdayDate
  AND (TimezoneId = $timezoneId OR TimezoneId IS NULL)
ORDER BY CreatedAtUtc, Id;"
    ))?;
    let rows = statement.query_map(
        named_params! {
            "$workdayDate": format_date(workday.date),
            "$timezoneId": workday.time_zone_id,
        },
        |row| {
            Ok(parse(
                &StoredRow::new(row, "ManualAdjustment"),
                &workday.time_zone_id,
            ))
        },
    )?;
    collect(rows)
}

fn find_by_id(
    connection: &Connection,
    id: &AdjustmentId,
    fallback_time_zone_id: &str,
) -> Result<Option<ManualAdjustment>, ManualAdjustmentError> {
    let found = connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM ManualAdjustment WHERE Id = $id;"),
            named_params! { "$id": id.as_str() },
            |row| {
                Ok(parse(
                    &StoredRow::new(row, "ManualAdjustment"),
                    fallback_time_zone_id,
                ))
            },
        )
        .optional()?;
    Ok(found.transpose()?)
}

fn find_successor_id(
    connection: &Connection,
    superseded_id: &AdjustmentId,
) -> Result<Option<AdjustmentId>, ManualAdjustmentError> {
    let found = connection
        .query_row(
            "SELECT Id FROM ManualAdjustment WHERE SupersedesId = $supersedesId LIMIT 1;",
            named_params! { "$supersedesId": superseded_id.as_str() },
            |row| {
                let row = StoredRow::new(row, "ManualAdjustment");
                Ok(row.text("Id").and_then(|id| canonical_id(&row, "Id", id)))
            },
        )
        .optional()?;
    Ok(found.transpose()?)
}

fn insert(connection: &Connection, record: &AdjustmentRecord) -> Result<(), ManualAdjustmentError> {
    connection.execute(
        "INSERT INTO ManualAdjustment (
    Id,
    WorkdayDate,
    Kind,
    StartUtc,
    EndUtc,
    DurationMinutes,
    FromLabel,
    ToLabel,
    Reason,
    CreatedAtUtc,
    SupersedesId,
    TimezoneId,
    ConfirmsReviewId)
VALUES (
    $id,
    $workdayDate,
    $kind,
    $startUtc,
    $endUtc,
    $durationMinutes,
    NULL,
    $toLabel,
    $reason,
    $createdAtUtc,
    $supersedesId,
    $timezoneId,
    $confirmsReviewId);",
        named_params! {
            "$id": record.id,
            "$workdayDate": record.workday_date,
            "$kind": record.kind,
            "$startUtc": record.start_utc,
            "$endUtc": record.end_utc,
            "$durationMinutes": record.duration_minutes,
            "$toLabel": record.to_label,
            "$reason": record.reason,
            "$createdAtUtc": record.created_at_utc,
            "$supersedesId": record.supersedes_id,
            "$timezoneId": record.timezone_id,
            "$confirmsReviewId": record.confirms_review_id,
        },
    )?;
    Ok(())
}

fn canonical_id(
    row: &StoredRow,
    column: &str,
    value: &str,
) -> Result<AdjustmentId, InvalidStoredValue> {
    AdjustmentId::new(value)
        .ok()
        .filter(|id| id.as_str() == value)
        .ok_or_else(|| row.invalid(format!("{column} is not canonical")))
}

fn parse(
    row: &StoredRow,
    fallback_time_zone_id: &str,
) -> Result<ManualAdjustment, InvalidStoredValue> {
    let id = canonical_id(row, "Id", row.text("Id")?)?;
    let date = row.date("WorkdayDate")?;
    let kind = row.text("Kind")?;
    let start = row.optional_instant("StartUtc")?;
    let end = row.optional_instant("EndUtc")?;
    let duration_minutes = row.optional_integer("DurationMinutes")?;
    let from_label = row.optional_text("FromLabel")?;
    let to_label = row.optional_text("ToLabel")?;
    let reason = row.optional_text("Reason")?;
    let created_at = row.instant("CreatedAtUtc")?;
    let supersedes_id = row
        .optional_text("SupersedesId")?
        .map(|value| canonical_id(row, "SupersedesId", value))
        .transpose()?;
    let time_zone_id = row.canonical_time_zone_id(
        "TimezoneId",
        row.optional_text("TimezoneId")?
            .unwrap_or(fallback_time_zone_id),
    )?;
    let confirms_review_id = row.optional_text("ConfirmsReviewId")?;
    if from_label.is_some() {
        return Err(row.invalid("FromLabel is not supported by the current adjustment model"));
    }

    let interval = |start: Timestamp, end: Timestamp| {
        UtcInterval::new(start, end)
            .map_err(|_| row.invalid("the interval must have a positive duration"))
    };
    let unrecognized =
        || row.invalid("the stored columns do not form a recognized adjustment variant");
    let kind = match (
        kind,
        start,
        end,
        duration_minutes,
        to_label,
        confirms_review_id,
    ) {
        ("add-work", Some(start), Some(end), None, None, None) => AdjustmentKind::IntervalWork {
            interval: interval(start, end)?,
        },
        ("add-work", None, None, Some(minutes), None, None) if minutes > 0 => {
            if minutes > i64::MAX / TICKS_PER_MINUTE {
                return Err(row.invalid("DurationMinutes does not fit in a .NET TimeSpan"));
            }
            AdjustmentKind::DurationWork {
                duration: SignedDuration::from_mins(minutes),
            }
        }
        ("exclude-time", Some(start), Some(end), None, None, None) => AdjustmentKind::Exclusion {
            interval: interval(start, end)?,
        },
        ("reclassify", Some(start), Some(end), None, Some(label), confirms_review_id) => {
            AdjustmentKind::Reclassification {
                interval: interval(start, end)?,
                classification: match label {
                    "credited" => ManualClassification::Credited,
                    "excluded" => ManualClassification::Excluded,
                    _ => {
                        return Err(
                            row.invalid("ToLabel is not a recognized manual classification")
                        );
                    }
                },
                confirms_review_id: confirms_review_id
                    .map(|value| {
                        LongLockReviewId::new(value)
                            .map_err(|_| row.invalid("ConfirmsReviewId is not canonical"))
                    })
                    .transpose()?,
            }
        }
        ("revoke", None, None, None, None, None) if supersedes_id.is_some() => {
            AdjustmentKind::Revocation
        }
        _ => return Err(unrecognized()),
    };
    let adjustment = ManualAdjustment::new(
        id,
        LocalWorkday::new(date, time_zone_id),
        created_at,
        supersedes_id,
        kind,
    )
    .map_err(|error| row.invalid(error.to_string()))?;
    Ok(match reason {
        Some(reason) => adjustment.with_reason(reason),
        None => adjustment,
    })
}

#[cfg(test)]
mod tests;
