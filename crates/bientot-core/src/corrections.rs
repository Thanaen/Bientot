//! Manual corrections of the current week, ported from
//! `Services/ManualAdjustmentCommandService.cs`. A correction is appended to the adjustment
//! history of its workday, validated against that day as accounted now; the adjustment id is the
//! command's idempotency key.

use std::collections::{HashMap, HashSet};

use bientot_domain::accounting::AdjustmentId;
use bientot_domain::adjustments::{
    AdjustmentKind, AdjustmentValidationError, ManualAdjustment, lineage,
};
use bientot_domain::time::LocalWorkday;
use bientot_store::adjustments::{
    AppendAdjustmentResult, ManualAdjustmentError, ManualAdjustmentRepository,
};

use crate::accounting::{AccountingError, WorkdayAccounting};
use crate::reviews::is_resolution_adjustment;
use crate::weekly::start_of_week;

#[derive(Debug)]
pub enum CorrectionError {
    /// The adjustment targets a day before the current local week, whose credit is closed.
    OutsideCurrentWeek,
    /// Includes [`AccountingError::FutureWorkday`] for a day after the current local day.
    Accounting(AccountingError),
    /// Includes [`ManualAdjustmentError::Validation`] for an adjustment the day's history
    /// refuses, and for the revocation of a long-lock review resolution.
    Adjustments(ManualAdjustmentError),
}

impl std::fmt::Display for CorrectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutsideCurrentWeek => {
                f.write_str("A manual adjustment must target the current local week.")
            }
            Self::Accounting(error) => error.fmt(f),
            Self::Adjustments(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for CorrectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OutsideCurrentWeek => None,
            Self::Accounting(error) => Some(error),
            Self::Adjustments(error) => Some(error),
        }
    }
}

impl From<AccountingError> for CorrectionError {
    fn from(error: AccountingError) -> Self {
        Self::Accounting(error)
    }
}

impl From<ManualAdjustmentError> for CorrectionError {
    fn from(error: ManualAdjustmentError) -> Self {
        Self::Adjustments(error)
    }
}

pub struct ManualAdjustmentCommands<'a> {
    accounting: &'a WorkdayAccounting<'a>,
    adjustments: ManualAdjustmentRepository<'a>,
}

impl<'a> ManualAdjustmentCommands<'a> {
    pub fn new(
        accounting: &'a WorkdayAccounting<'a>,
        adjustments: ManualAdjustmentRepository<'a>,
    ) -> Self {
        Self {
            accounting,
            adjustments,
        }
    }

    /// Appends `adjustment` to its workday of the current local week. A retry of the same
    /// adjustment finds it already recorded, whenever it runs in that week.
    pub fn save(
        &self,
        adjustment: &ManualAdjustment,
    ) -> Result<AppendAdjustmentResult, CorrectionError> {
        let mut saved = self.save_all(std::slice::from_ref(adjustment))?;
        Ok(saved.remove(0))
    }

    /// Appends `adjustments`, all of one workday of the current local week, in one write: each
    /// is validated after the ones before it, and none is recorded unless all are.
    pub fn save_all(
        &self,
        adjustments: &[ManualAdjustment],
    ) -> Result<Vec<AppendAdjustmentResult>, CorrectionError> {
        let Some(first) = adjustments.first() else {
            return Ok(Vec::new());
        };
        let workday = first.workday();
        assert!(
            adjustments
                .iter()
                .all(|adjustment| adjustment.workday() == workday),
            "a batch of corrections holds one workday"
        );
        let context = self.accounting.capture_context();
        let today = context.calendar().workday(context.observed_at).date;
        if workday.date < start_of_week(today) {
            return Err(CorrectionError::OutsideCurrentWeek);
        }
        for adjustment in adjustments {
            if let (AdjustmentKind::Revocation, Some(revoked_id)) =
                (adjustment.kind(), adjustment.supersedes_id())
            {
                self.ensure_revocable(adjustment, revoked_id)?;
            }
        }

        let target = self.accounting.calculate(workday.date, &context)?;
        Ok(self
            .adjustments
            .append_all_validated(target.summary_input.accounting(), adjustments)?)
    }

    /// The most recently created adjustment of `workday` that is still in effect, revocations
    /// aside: the one an edit replaces.
    pub fn latest_active(
        &self,
        workday: &LocalWorkday,
    ) -> Result<Option<ManualAdjustment>, ManualAdjustmentError> {
        let history = self.adjustments.read_for_workday(workday)?;
        let superseded = superseded_ids(&history);
        Ok(history
            .into_iter()
            .filter(|adjustment| {
                !matches!(adjustment.kind(), AdjustmentKind::Revocation)
                    && !superseded.contains(adjustment.id())
            })
            .max_by(|left, right| {
                (left.created_at(), left.id()).cmp(&(right.created_at(), right.id()))
            }))
    }

    /// The adjustment `id` of `workday`, unless a later adjustment superseded it.
    pub fn active(
        &self,
        workday: &LocalWorkday,
        id: &AdjustmentId,
    ) -> Result<Option<ManualAdjustment>, ManualAdjustmentError> {
        let history = self.adjustments.read_for_workday(workday)?;
        let superseded = superseded_ids(&history);
        Ok(history
            .into_iter()
            .find(|adjustment| adjustment.id() == id && !superseded.contains(id)))
    }

    /// Refuses the revocation of an adjustment bound to a long-lock review, or of a revocation:
    /// only a [`Binding::Free`] one is revoked. A target the history does not hold is left to its
    /// validation, which refuses it as missing.
    fn ensure_revocable(
        &self,
        revocation: &ManualAdjustment,
        revoked_id: &AdjustmentId,
    ) -> Result<(), CorrectionError> {
        let history = history_by_id(self.adjustments.read_for_workday(revocation.workday())?);
        let irrevocable = history
            .get(revoked_id)
            .is_some_and(|target| binding(target, &history) != Binding::Free);
        if irrevocable {
            return Err(ManualAdjustmentError::Validation(
                AdjustmentValidationError::InvalidRevocationTarget {
                    revocation_id: revocation.id().clone(),
                    target_id: revoked_id.clone(),
                },
            )
            .into());
        }
        Ok(())
    }
}

/// The adjustments of a workday by id, as [`binding`] reads them.
pub fn history_by_id(
    history: impl IntoIterator<Item = ManualAdjustment>,
) -> HashMap<AdjustmentId, ManualAdjustment> {
    history
        .into_iter()
        .map(|adjustment| (adjustment.id().clone(), adjustment))
        .collect()
}

/// How an adjustment of a workday's history stands to the long-lock reviews of that day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    /// Bound to no review: the ledger withdraws it.
    Free,
    /// A long-lock resolution, or an edit of a resolution or of a confirmation: it stays bound
    /// to the review's claim, which cannot be removed, although only the original names it.
    FromResolution,
    /// A long-lock confirmation, which names the review it resolves.
    ConfirmsReview,
    /// A revocation, which only withdraws another adjustment.
    Revocation,
}

/// How `adjustment` stands within its workday's `history`, see [`Binding`].
pub fn binding(
    adjustment: &ManualAdjustment,
    history: &HashMap<AdjustmentId, ManualAdjustment>,
) -> Binding {
    if *adjustment.kind() == AdjustmentKind::Revocation {
        Binding::Revocation
    } else if adjustment.kind().confirmed_review().is_some() {
        Binding::ConfirmsReview
    } else if lineage(adjustment, history)
        .any(|item| is_resolution_adjustment(item.id()) || item.kind().confirmed_review().is_some())
    {
        Binding::FromResolution
    } else {
        Binding::Free
    }
}

fn superseded_ids(history: &[ManualAdjustment]) -> HashSet<AdjustmentId> {
    history
        .iter()
        .filter_map(|adjustment| adjustment.supersedes_id().cloned())
        .collect()
}

#[cfg(test)]
mod tests;
