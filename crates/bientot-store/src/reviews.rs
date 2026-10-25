//! Mutually exclusive resolutions of long-lock reviews, ported from
//! `Reviews/LongLockResolutionRepository.cs` and the types of `ILongLockResolutionStore.cs`.

use bientot_domain::accounting::{BlankIdentifier, LongLockReviewId};
use bientot_domain::time::LocalWorkday;
use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, named_params};

use crate::codec::{format_date, format_instant, truncate_to_tick};
use crate::row::{InvalidStoredValue, StorageError, StoredRow, storage_error_conversions};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LongLockResolutionActionKind {
    ConfirmWork,
    Exclude,
    FinishAtLockStart,
}

impl LongLockResolutionActionKind {
    const ALL: [Self; 3] = [Self::ConfirmWork, Self::Exclude, Self::FinishAtLockStart];

    fn storage(self) -> &'static str {
        match self {
            Self::ConfirmWork => "confirm-work",
            Self::Exclude => "exclude",
            Self::FinishAtLockStart => "finish-at-lock-start",
        }
    }
}

/// The command that resolves a review: its kind and the identity of that one command.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LongLockResolutionAction {
    kind: LongLockResolutionActionKind,
    identity: String,
}

impl LongLockResolutionAction {
    pub fn new(
        kind: LongLockResolutionActionKind,
        identity: &str,
    ) -> Result<Self, BlankIdentifier> {
        let identity = identity.trim();
        if identity.is_empty() {
            return Err(BlankIdentifier);
        }
        Ok(Self {
            kind,
            identity: identity.to_owned(),
        })
    }

    pub fn kind(&self) -> LongLockResolutionActionKind {
        self.kind
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LongLockResolutionClaim {
    pub workday: LocalWorkday,
    pub review_id: LongLockReviewId,
    pub action: LongLockResolutionAction,
    pub proposed_created_at: Timestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimLongLockResolutionOutcome {
    Acquired,
    AlreadyClaimed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClaimLongLockResolutionResult {
    pub outcome: ClaimLongLockResolutionOutcome,
    /// The creation instant of the first claim, which every retry of it reuses.
    pub canonical_created_at: Timestamp,
}

/// The review was already resolved by another action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LongLockResolutionConflict {
    pub workday: LocalWorkday,
    pub review_id: LongLockReviewId,
    pub attempted_action: LongLockResolutionAction,
    pub existing_action: LongLockResolutionAction,
}

#[derive(Debug)]
pub enum LongLockResolutionError {
    Storage(StorageError),
    Conflict(Box<LongLockResolutionConflict>),
}

impl std::fmt::Display for LongLockResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(f),
            Self::Conflict(conflict) => write!(
                f,
                "Long-lock review '{}' for {} in '{}' was already resolved by '{:?}'.",
                conflict.review_id,
                format_date(conflict.workday.date),
                conflict.workday.time_zone_id,
                conflict.existing_action.kind
            ),
        }
    }
}

impl std::error::Error for LongLockResolutionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::Conflict(_) => None,
        }
    }
}

storage_error_conversions!(LongLockResolutionError);

pub struct LongLockResolutionRepository<'a> {
    connection: &'a Connection,
}

impl<'a> LongLockResolutionRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Claims the review of `claim` for its action. A retry of the same action finds the claim
    /// already held; any other action conflicts with it.
    pub fn claim(
        &self,
        claim: &LongLockResolutionClaim,
    ) -> Result<ClaimLongLockResolutionResult, LongLockResolutionError> {
        let transaction = crate::database::scope(self.connection, TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT ActionKind, ActionIdentity, CanonicalCreatedAtUtc
FROM LongLockResolution
WHERE WorkdayDate = $workdayDate
  AND TimezoneId = $timezoneId
  AND ReviewId = $reviewId;",
                named_params! {
                    "$workdayDate": format_date(claim.workday.date),
                    "$timezoneId": claim.workday.time_zone_id,
                    "$reviewId": claim.review_id.as_str(),
                },
                |row| Ok(parse(&StoredRow::new(row, "LongLockResolution"))),
            )
            .optional()?
            .transpose()?;
        if let Some((existing_action, canonical_created_at)) = existing {
            if existing_action != claim.action {
                return Err(LongLockResolutionError::Conflict(Box::new(
                    LongLockResolutionConflict {
                        workday: claim.workday.clone(),
                        review_id: claim.review_id.clone(),
                        attempted_action: claim.action.clone(),
                        existing_action,
                    },
                )));
            }
            transaction.commit()?;
            return Ok(ClaimLongLockResolutionResult {
                outcome: ClaimLongLockResolutionOutcome::AlreadyClaimed,
                canonical_created_at,
            });
        }

        transaction.execute(
            "INSERT INTO LongLockResolution (
    WorkdayDate,
    TimezoneId,
    ReviewId,
    ActionKind,
    ActionIdentity,
    CanonicalCreatedAtUtc)
VALUES (
    $workdayDate,
    $timezoneId,
    $reviewId,
    $actionKind,
    $actionIdentity,
    $canonicalCreatedAtUtc);",
            named_params! {
                "$workdayDate": format_date(claim.workday.date),
                "$timezoneId": claim.workday.time_zone_id,
                "$reviewId": claim.review_id.as_str(),
                "$actionKind": claim.action.kind.storage(),
                "$actionIdentity": claim.action.identity,
                "$canonicalCreatedAtUtc": format_instant(claim.proposed_created_at),
            },
        )?;
        transaction.commit()?;
        Ok(ClaimLongLockResolutionResult {
            outcome: ClaimLongLockResolutionOutcome::Acquired,
            canonical_created_at: truncate_to_tick(claim.proposed_created_at),
        })
    }

    /// Deletes the claim when it is held by exactly this action; returns whether it was.
    pub fn release(
        &self,
        claim: &LongLockResolutionClaim,
    ) -> Result<bool, LongLockResolutionError> {
        Ok(delete(self.connection, claim, None)?)
    }
}

/// Deletes the claim when it is held by exactly this action and, given a `generation`, was
/// acquired with that canonical instant; returns whether it was.
pub(crate) fn delete(
    connection: &Connection,
    claim: &LongLockResolutionClaim,
    generation: Option<Timestamp>,
) -> rusqlite::Result<bool> {
    let deleted = connection.execute(
        "DELETE FROM LongLockResolution
WHERE WorkdayDate = $workdayDate
  AND TimezoneId = $timezoneId
  AND ReviewId = $reviewId
  AND ActionKind = $actionKind
  AND ActionIdentity = $actionIdentity
  AND ($generation IS NULL OR CanonicalCreatedAtUtc = $generation);",
        named_params! {
            "$workdayDate": format_date(claim.workday.date),
            "$timezoneId": claim.workday.time_zone_id,
            "$reviewId": claim.review_id.as_str(),
            "$actionKind": claim.action.kind.storage(),
            "$actionIdentity": claim.action.identity,
            "$generation": generation.map(format_instant),
        },
    )?;
    Ok(deleted == 1)
}

/// Whether exactly this action still holds its claim, acquired with the canonical instant
/// `generation`, read in `transaction`. A claim released and acquired again by the same action
/// carries a later instant, so a caller of the earlier one no longer holds it.
pub(crate) fn is_held(
    transaction: &Connection,
    claim: &LongLockResolutionClaim,
    generation: Timestamp,
) -> rusqlite::Result<bool> {
    Ok(transaction
        .query_row(
            "SELECT 1
FROM LongLockResolution
WHERE WorkdayDate = $workdayDate
  AND TimezoneId = $timezoneId
  AND ReviewId = $reviewId
  AND ActionKind = $actionKind
  AND ActionIdentity = $actionIdentity
  AND CanonicalCreatedAtUtc = $generation;",
            named_params! {
                "$workdayDate": format_date(claim.workday.date),
                "$timezoneId": claim.workday.time_zone_id,
                "$reviewId": claim.review_id.as_str(),
                "$actionKind": claim.action.kind.storage(),
                "$actionIdentity": claim.action.identity,
                "$generation": format_instant(generation),
            },
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn parse(row: &StoredRow) -> Result<(LongLockResolutionAction, Timestamp), InvalidStoredValue> {
    let kind_text = row.text("ActionKind")?;
    let kind = LongLockResolutionActionKind::ALL
        .into_iter()
        .find(|kind| kind.storage() == kind_text)
        .ok_or_else(|| row.invalid("ActionKind is not a recognized action kind"))?;
    let action = LongLockResolutionAction::new(kind, row.text("ActionIdentity")?)
        .map_err(|_| row.invalid("ActionIdentity is blank"))?;
    Ok((action, row.instant("CanonicalCreatedAtUtc")?))
}

#[cfg(test)]
mod tests;
