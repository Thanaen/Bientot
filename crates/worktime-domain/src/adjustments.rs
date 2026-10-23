//! Manual adjustments: their validation as an append-only history and their application to the
//! automatic timeline.

use std::collections::{BTreeSet, HashMap, HashSet};

use jiff::{SignedDuration, Timestamp};

use crate::accounting::model::append_canonical;
use crate::accounting::{
    AccountedOrigin, AccountedSegment, AccountingLabel, AccountingResult, AccountingTimeline,
    AdjustmentId, LongLockReviewId, ManualDurationCredit,
};
use crate::intervals::UtcInterval;
use crate::time::LocalWorkday;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ManualClassification {
    Credited,
    Excluded,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AdjustmentKind {
    IntervalWork {
        interval: UtcInterval,
    },
    DurationWork {
        duration: SignedDuration,
    },
    Exclusion {
        interval: UtcInterval,
    },
    Reclassification {
        interval: UtcInterval,
        classification: ManualClassification,
        confirms_review_id: Option<LongLockReviewId>,
    },
    /// Withdraws the adjustment it supersedes without replacing it, restoring the automatic
    /// accounting for that span. It has no accounting effect of its own.
    Revocation,
}

impl AdjustmentKind {
    /// The long-lock review a confirmation resolves; `None` for any other adjustment.
    pub fn confirmed_review(&self) -> Option<&LongLockReviewId> {
        match self {
            Self::Reclassification {
                confirms_review_id, ..
            } => confirms_review_id.as_ref(),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidAdjustment {
    NotAWorkingDay,
    NonPositiveDuration,
    ConfirmationMustCredit,
    RevocationWithoutTarget,
}

impl std::fmt::Display for InvalidAdjustment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotAWorkingDay => "an adjustment must belong to a Monday-Friday workday",
            Self::NonPositiveDuration => "a duration adjustment must be positive",
            Self::ConfirmationMustCredit => {
                "a long-lock confirmation must classify the interval as credited"
            }
            Self::RevocationWithoutTarget => "a revocation must name the adjustment it revokes",
        })
    }
}

impl std::error::Error for InvalidAdjustment {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ManualAdjustment {
    id: AdjustmentId,
    workday: LocalWorkday,
    created_at: Timestamp,
    supersedes_id: Option<AdjustmentId>,
    reason: Option<String>,
    kind: AdjustmentKind,
}

impl ManualAdjustment {
    pub fn interval_work(
        id: AdjustmentId,
        workday: LocalWorkday,
        interval: UtcInterval,
        created_at: Timestamp,
    ) -> Result<Self, InvalidAdjustment> {
        Self::new(
            id,
            workday,
            created_at,
            None,
            AdjustmentKind::IntervalWork { interval },
        )
    }

    pub fn duration_work(
        id: AdjustmentId,
        workday: LocalWorkday,
        duration: SignedDuration,
        created_at: Timestamp,
    ) -> Result<Self, InvalidAdjustment> {
        if !duration.is_positive() {
            return Err(InvalidAdjustment::NonPositiveDuration);
        }
        Self::new(
            id,
            workday,
            created_at,
            None,
            AdjustmentKind::DurationWork { duration },
        )
    }

    pub fn exclusion(
        id: AdjustmentId,
        workday: LocalWorkday,
        interval: UtcInterval,
        created_at: Timestamp,
    ) -> Result<Self, InvalidAdjustment> {
        Self::new(
            id,
            workday,
            created_at,
            None,
            AdjustmentKind::Exclusion { interval },
        )
    }

    pub fn reclassification(
        id: AdjustmentId,
        workday: LocalWorkday,
        interval: UtcInterval,
        classification: ManualClassification,
        created_at: Timestamp,
    ) -> Result<Self, InvalidAdjustment> {
        Self::new(
            id,
            workday,
            created_at,
            None,
            AdjustmentKind::Reclassification {
                interval,
                classification,
                confirms_review_id: None,
            },
        )
    }

    /// A credited reclassification that resolves the long-lock review `review_id`.
    pub fn long_lock_confirmation(
        id: AdjustmentId,
        workday: LocalWorkday,
        interval: UtcInterval,
        created_at: Timestamp,
        review_id: LongLockReviewId,
    ) -> Result<Self, InvalidAdjustment> {
        Self::new(
            id,
            workday,
            created_at,
            None,
            AdjustmentKind::Reclassification {
                interval,
                classification: ManualClassification::Credited,
                confirms_review_id: Some(review_id),
            },
        )
    }

    pub fn revocation(
        id: AdjustmentId,
        workday: LocalWorkday,
        revokes_id: AdjustmentId,
        created_at: Timestamp,
    ) -> Result<Self, InvalidAdjustment> {
        Self::new(
            id,
            workday,
            created_at,
            Some(revokes_id),
            AdjustmentKind::Revocation,
        )
    }

    /// Any kind of adjustment, for callers that restore one from storage.
    pub fn new(
        id: AdjustmentId,
        workday: LocalWorkday,
        created_at: Timestamp,
        supersedes_id: Option<AdjustmentId>,
        kind: AdjustmentKind,
    ) -> Result<Self, InvalidAdjustment> {
        if !workday.is_working_day() {
            return Err(InvalidAdjustment::NotAWorkingDay);
        }
        match &kind {
            AdjustmentKind::DurationWork { duration } if !duration.is_positive() => {
                return Err(InvalidAdjustment::NonPositiveDuration);
            }
            AdjustmentKind::Reclassification {
                classification: ManualClassification::Excluded,
                confirms_review_id: Some(_),
                ..
            } => return Err(InvalidAdjustment::ConfirmationMustCredit),
            AdjustmentKind::Revocation if supersedes_id.is_none() => {
                return Err(InvalidAdjustment::RevocationWithoutTarget);
            }
            _ => {}
        }
        Ok(Self {
            id,
            workday,
            created_at,
            supersedes_id,
            reason: None,
            kind,
        })
    }

    pub fn superseding(self, supersedes_id: AdjustmentId) -> Self {
        Self {
            supersedes_id: Some(supersedes_id),
            ..self
        }
    }

    /// A blank reason is no reason.
    pub fn with_reason(self, reason: &str) -> Self {
        let reason = reason.trim();
        Self {
            reason: (!reason.is_empty()).then(|| reason.to_owned()),
            ..self
        }
    }

    pub fn id(&self) -> &AdjustmentId {
        &self.id
    }

    pub fn workday(&self) -> &LocalWorkday {
        &self.workday
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn supersedes_id(&self) -> Option<&AdjustmentId> {
        self.supersedes_id.as_ref()
    }

    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    pub fn kind(&self) -> &AdjustmentKind {
        &self.kind
    }

    pub fn interval(&self) -> Option<UtcInterval> {
        match self.kind {
            AdjustmentKind::IntervalWork { interval }
            | AdjustmentKind::Exclusion { interval }
            | AdjustmentKind::Reclassification { interval, .. } => Some(interval),
            AdjustmentKind::DurationWork { .. } | AdjustmentKind::Revocation => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdjustmentValidationError {
    ConflictingId {
        id: AdjustmentId,
    },
    MissingSupersededAdjustment {
        id: AdjustmentId,
        superseded_id: AdjustmentId,
    },
    InvalidSupersessionOrder {
        id: AdjustmentId,
    },
    MultipleActiveSuccessors,
    WrongWorkday {
        id: AdjustmentId,
    },
    OutsideAccountingHorizon {
        id: AdjustmentId,
    },
    OverlappingActiveAdjustments,
    OverlapsCreditedCoverage {
        id: AdjustmentId,
    },
    UnknownLongLockReview {
        review_id: LongLockReviewId,
    },
    IncompleteLongLockConfirmation {
        review_id: LongLockReviewId,
    },
    InvalidRevocationTarget {
        revocation_id: AdjustmentId,
        target_id: AdjustmentId,
    },
}

impl std::fmt::Display for AdjustmentValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConflictingId { id } => {
                write!(
                    f,
                    "adjustment ID '{id}' identifies two different adjustments"
                )
            }
            Self::MissingSupersededAdjustment { id, superseded_id } => write!(
                f,
                "adjustment '{id}' supersedes unknown adjustment '{superseded_id}'"
            ),
            Self::InvalidSupersessionOrder { id } => write!(
                f,
                "adjustment '{id}' must be created after the adjustment it supersedes"
            ),
            Self::MultipleActiveSuccessors => {
                f.write_str("a supersession chain can have only one active successor")
            }
            Self::WrongWorkday { id } => {
                write!(f, "adjustment '{id}' belongs to another workday")
            }
            Self::OutsideAccountingHorizon { id } => write!(
                f,
                "adjustment '{id}' lies outside the current accounting horizon"
            ),
            Self::OverlappingActiveAdjustments => {
                f.write_str("active interval adjustments must not overlap")
            }
            Self::OverlapsCreditedCoverage { id } => write!(
                f,
                "interval work adjustment '{id}' overlaps already credited coverage"
            ),
            Self::UnknownLongLockReview { review_id } => {
                write!(f, "long-lock review '{review_id}' is not pending")
            }
            Self::IncompleteLongLockConfirmation { review_id } => write!(
                f,
                "confirmation for long-lock review '{review_id}' must cover its complete current \
                 accounting portion"
            ),
            Self::InvalidRevocationTarget {
                revocation_id,
                target_id,
            } => write!(
                f,
                "revocation '{revocation_id}' cannot revoke adjustment '{target_id}'"
            ),
        }
    }
}

impl std::error::Error for AdjustmentValidationError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdjustmentApplication {
    pub timeline: AccountingTimeline,
    pub duration_credits: Vec<ManualDurationCredit>,
}

/// Checks that appending `candidate` to `adjustment_history` keeps a valid history for the
/// workday that `current` accounts.
pub fn validate_for_append(
    current: &AccountingResult,
    adjustment_history: &[ManualAdjustment],
    candidate: &ManualAdjustment,
) -> Result<(), AdjustmentValidationError> {
    let workday = &current.timeline().window().workday;
    let existing = History::normalize(adjustment_history)?;
    existing.validate(workday)?;
    if let Some(duplicate) = existing.get(&candidate.id) {
        if duplicate != candidate {
            return Err(AdjustmentValidationError::ConflictingId {
                id: candidate.id.clone(),
            });
        }
        return validate_structure(current.timeline(), &existing.active());
    }

    let appended: Vec<_> = existing.adjustments().chain([candidate]).cloned().collect();
    let prospective = History::normalize(&appended)?;
    prospective.validate(workday)?;
    validate_structure(current.timeline(), &prospective.active())?;
    validate_candidate(current, &existing, candidate)
}

/// The intervals the active adjustments of `adjustment_history` set; none for a history
/// [`apply`] rejects.
pub fn active_intervals(adjustment_history: &[ManualAdjustment]) -> Vec<UtcInterval> {
    History::normalize(adjustment_history).map_or_else(
        |_| Vec::new(),
        |history| {
            history
                .active()
                .into_iter()
                .filter_map(ManualAdjustment::interval)
                .collect()
        },
    )
}

pub fn apply(
    automatic: AccountingTimeline,
    adjustment_history: &[ManualAdjustment],
) -> Result<AdjustmentApplication, AdjustmentValidationError> {
    let history = History::normalize(adjustment_history)?;
    history.validate(&automatic.window().workday)?;
    let active = history.active();
    validate_structure(&automatic, &active)?;

    let mut interval_adjustments: Vec<_> = active
        .iter()
        .copied()
        .filter_map(|adjustment| Some((adjustment, adjustment.interval()?)))
        .collect();
    interval_adjustments.sort_by(|(left, left_interval), (right, right_interval)| {
        (left_interval.start(), left.created_at, &left.id).cmp(&(
            right_interval.start(),
            right.created_at,
            &right.id,
        ))
    });
    let duration_credits = active
        .iter()
        .filter_map(|adjustment| match adjustment.kind {
            AdjustmentKind::DurationWork { duration } => {
                Some(ManualDurationCredit::new(adjustment.id.clone(), duration))
            }
            _ => None,
        })
        .collect();
    if interval_adjustments.is_empty() {
        return Ok(AdjustmentApplication {
            timeline: automatic,
            duration_credits,
        });
    }

    let segments = apply_intervals(&automatic, &interval_adjustments);
    Ok(AdjustmentApplication {
        timeline: AccountingTimeline::new(
            automatic.window().clone(),
            automatic.through(),
            segments,
        ),
        duration_credits,
    })
}

/// The adjustment followed by every adjustment it transitively supersedes.
pub fn lineage<'a>(
    adjustment: &'a ManualAdjustment,
    history: &'a HashMap<AdjustmentId, ManualAdjustment>,
) -> impl Iterator<Item = &'a ManualAdjustment> {
    std::iter::successors(Some(adjustment), |current| {
        history.get(current.supersedes_id.as_ref()?)
    })
}

/// Adjustments by id in first-seen order, which decides the error reported first, as in .NET.
struct History<'a> {
    order: Vec<&'a ManualAdjustment>,
    by_id: HashMap<&'a AdjustmentId, &'a ManualAdjustment>,
}

impl<'a> History<'a> {
    fn normalize(adjustments: &'a [ManualAdjustment]) -> Result<Self, AdjustmentValidationError> {
        let mut history = Self {
            order: Vec::new(),
            by_id: HashMap::new(),
        };
        for adjustment in adjustments {
            match history.by_id.get(&adjustment.id) {
                Some(&existing) if existing != adjustment => {
                    return Err(AdjustmentValidationError::ConflictingId {
                        id: adjustment.id.clone(),
                    });
                }
                Some(_) => {}
                None => {
                    history.by_id.insert(&adjustment.id, adjustment);
                    history.order.push(adjustment);
                }
            }
        }
        Ok(history)
    }

    fn get(&self, id: &AdjustmentId) -> Option<&'a ManualAdjustment> {
        self.by_id.get(id).copied()
    }

    fn adjustments(&self) -> impl Iterator<Item = &'a ManualAdjustment> + '_ {
        self.order.iter().copied()
    }

    fn superseded_by(&self, adjustment: &ManualAdjustment) -> Option<&'a ManualAdjustment> {
        self.get(adjustment.supersedes_id.as_ref()?)
    }

    fn lineage(
        &self,
        adjustment: &'a ManualAdjustment,
    ) -> impl Iterator<Item = &'a ManualAdjustment> + '_ {
        std::iter::successors(Some(adjustment), |current| self.superseded_by(current))
    }

    fn validate(&self, workday: &LocalWorkday) -> Result<(), AdjustmentValidationError> {
        for adjustment in self.adjustments() {
            if adjustment.workday != *workday {
                return Err(AdjustmentValidationError::WrongWorkday {
                    id: adjustment.id.clone(),
                });
            }
            let Some(superseded_id) = &adjustment.supersedes_id else {
                continue;
            };
            let Some(superseded) = self.get(superseded_id) else {
                return Err(AdjustmentValidationError::MissingSupersededAdjustment {
                    id: adjustment.id.clone(),
                    superseded_id: superseded_id.clone(),
                });
            };
            if adjustment.created_at <= superseded.created_at {
                return Err(AdjustmentValidationError::InvalidSupersessionOrder {
                    id: adjustment.id.clone(),
                });
            }
        }

        for revocation in self
            .adjustments()
            .filter(|adjustment| adjustment.kind == AdjustmentKind::Revocation)
        {
            let target = self
                .superseded_by(revocation)
                .expect("a revocation always supersedes a known adjustment");
            // A long-lock confirmation, and every later edit of it, is bound to a resolution
            // record that cannot be withdrawn.
            if target.kind == AdjustmentKind::Revocation
                || self
                    .lineage(target)
                    .any(|item| item.kind.confirmed_review().is_some())
            {
                return Err(AdjustmentValidationError::InvalidRevocationTarget {
                    revocation_id: revocation.id.clone(),
                    target_id: target.id.clone(),
                });
            }
        }

        let mut roots = HashSet::new();
        for adjustment in self.active_unordered() {
            let root = self
                .lineage(adjustment)
                .last()
                .expect("a lineage starts with the adjustment itself");
            if !roots.insert(&root.id) {
                return Err(AdjustmentValidationError::MultipleActiveSuccessors);
            }
        }
        Ok(())
    }

    fn active_unordered(&self) -> impl Iterator<Item = &'a ManualAdjustment> + '_ {
        let superseded: HashSet<_> = self
            .adjustments()
            .filter_map(|adjustment| adjustment.supersedes_id.as_ref())
            .collect();
        self.adjustments()
            .filter(move |adjustment| !superseded.contains(&adjustment.id))
    }

    /// The adjustments no other adjustment supersedes, by creation time then id.
    fn active(&self) -> Vec<&'a ManualAdjustment> {
        let mut active: Vec<_> = self.active_unordered().collect();
        active
            .sort_by(|left, right| (left.created_at, &left.id).cmp(&(right.created_at, &right.id)));
        active
    }
}

fn validate_structure(
    timeline: &AccountingTimeline,
    active: &[&ManualAdjustment],
) -> Result<(), AdjustmentValidationError> {
    let mut intervals: Vec<_> = active
        .iter()
        .filter_map(|adjustment| Some((*adjustment, adjustment.interval()?)))
        .collect();
    intervals.sort_by_key(|(_, interval)| (interval.start(), interval.end()));

    for (index, &(adjustment, interval)) in intervals.iter().enumerate() {
        if interval.start() < timeline.window().start || interval.end() > timeline.through() {
            return Err(AdjustmentValidationError::OutsideAccountingHorizon {
                id: adjustment.id.clone(),
            });
        }
        if index > 0 && intervals[index - 1].1.overlaps(interval) {
            return Err(AdjustmentValidationError::OverlappingActiveAdjustments);
        }
    }
    Ok(())
}

fn validate_candidate(
    current: &AccountingResult,
    history: &History<'_>,
    candidate: &ManualAdjustment,
) -> Result<(), AdjustmentValidationError> {
    let Some(interval) = candidate.interval() else {
        return Ok(());
    };

    if matches!(candidate.kind, AdjustmentKind::IntervalWork { .. }) {
        let superseded: HashSet<_> =
            std::iter::successors(candidate.supersedes_id.as_ref(), |id| {
                history
                    .get(id)
                    .and_then(|adjustment| adjustment.supersedes_id.as_ref())
            })
            .collect();
        let overlaps_credit = current.timeline().segments().iter().any(|segment| {
            segment.kind().is_credit()
                && segment.interval().overlaps(interval)
                && !segment
                    .origin()
                    .manual_adjustment_id()
                    .is_some_and(|id| superseded.contains(id))
        });
        if overlaps_credit {
            return Err(AdjustmentValidationError::OverlapsCreditedCoverage {
                id: candidate.id.clone(),
            });
        }
    }

    let Some(review_id) = candidate.kind.confirmed_review() else {
        return Ok(());
    };
    let Some(review) = current
        .pending_reviews()
        .iter()
        .find(|review| review.id == *review_id)
    else {
        return Err(AdjustmentValidationError::UnknownLongLockReview {
            review_id: review_id.clone(),
        });
    };
    let timeline = current.timeline();
    let expected = UtcInterval::new(timeline.window().start, timeline.through())
        .ok()
        .and_then(|horizon| review.locked_episode.intersect(horizon));
    if expected != Some(interval) {
        return Err(AdjustmentValidationError::IncompleteLongLockConfirmation {
            review_id: review_id.clone(),
        });
    }
    Ok(())
}

fn apply_intervals(
    timeline: &AccountingTimeline,
    adjustments: &[(&ManualAdjustment, UtcInterval)],
) -> Vec<AccountedSegment> {
    let mut segments = Vec::new();
    for automatic in timeline.segments() {
        let bounds = automatic.interval();
        let inner = adjustments
            .iter()
            .flat_map(|(_, interval)| [interval.start(), interval.end()])
            .filter(|&point| bounds.start() < point && point < bounds.end());
        let points: Vec<_> = [bounds.start(), bounds.end()]
            .into_iter()
            .chain(inner)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for pair in points.windows(2) {
            let interval =
                UtcInterval::new(pair[0], pair[1]).expect("cut points are strictly sorted");
            let adjustment = adjustments
                .iter()
                .find(|(_, adjusted)| adjusted.contains(interval.start()));
            let next = match adjustment {
                None => automatic.with_interval(interval),
                Some((adjustment, _)) => apply_adjustment(automatic, interval, adjustment),
            };
            append_canonical(&mut segments, next);
        }
    }
    segments
}

fn apply_adjustment(
    automatic: &AccountedSegment,
    interval: UtcInterval,
    adjustment: &ManualAdjustment,
) -> AccountedSegment {
    // Sleep inside a lock is no work, while an appointment's absence was confirmed asleep or
    // not.
    if automatic.label() == AccountingLabel::Suspended
        && adjustment
            .kind
            .confirmed_review()
            .is_some_and(|review| review.appointment_id().is_none())
    {
        return automatic.with_interval(interval);
    }

    let origin = AccountedOrigin::Manual {
        adjustment_id: adjustment.id.clone(),
        replaces: Box::new(automatic.origin().clone()),
    };
    let credited = match &adjustment.kind {
        AdjustmentKind::IntervalWork { .. } => true,
        AdjustmentKind::Exclusion { .. } => false,
        AdjustmentKind::Reclassification { classification, .. } => {
            *classification == ManualClassification::Credited
        }
        AdjustmentKind::DurationWork { .. } | AdjustmentKind::Revocation => {
            unreachable!("only interval adjustments reach the timeline")
        }
    };
    if credited {
        AccountedSegment::credited(interval, AccountingLabel::Manual, origin)
    } else {
        AccountedSegment::excluded(interval, AccountingLabel::Manual, origin)
    }
}

#[cfg(test)]
mod tests;
