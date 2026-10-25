//! Long-lock reviews, ported from `Services/LongLockReviewService.cs` and
//! `Services/LongLockReviewCommandService.cs`. A review is resolved once: the first action claims
//! it in the ledger, then records its adjustment or its finish. Every action has a deterministic
//! identity, so a retry finds its own claim and its own record.

use bientot_domain::accounting::{
    AccountingResult, AdjustmentId, LongLockReview, LongLockReviewId, LongLockReviewReason,
    OperationKey, RecordedTransition, TrackingState, WorkdayTransition, WorkdayTransitionKind,
};
use bientot_domain::adjustments::{ManualAdjustment, ManualClassification};
use bientot_domain::intervals::UtcInterval;
use bientot_domain::time::{LocalWorkday, WorkdayWindow};
use bientot_store::adjustments::{
    AppendAdjustmentOutcome, AppendAdjustmentResult, ManualAdjustmentError,
    ManualAdjustmentRepository,
};
use bientot_store::codec::{format_date, format_instant, truncate_to_tick};
use bientot_store::reviews::{
    LongLockResolutionAction, LongLockResolutionActionKind, LongLockResolutionClaim,
    LongLockResolutionError, LongLockResolutionRepository,
};
use bientot_store::sessions::{RecordTransitionResult, SessionEventError, SessionEventRepository};
use jiff::Timestamp;

use crate::accounting::{AccountingError, WorkdayAccounting};
use crate::weekly::start_of_week;

const FINISH_OPERATION_KEY_PREFIX: &str = "long-lock-review";
const RESOLUTION_ADJUSTMENT_ID_PREFIX: &str = "long-lock-review:";
const IDENTITY_SEPARATOR: &str = "\u{1F}";

/// A resolution recorded as an adjustment, as the history screen offers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ManualResolution {
    ConfirmWork,
    Exclude,
}

impl ManualResolution {
    fn action_kind(self) -> LongLockResolutionActionKind {
        match self {
            Self::ConfirmWork => LongLockResolutionActionKind::ConfirmWork,
            Self::Exclude => LongLockResolutionActionKind::Exclude,
        }
    }

    fn classification(self) -> ManualClassification {
        match self {
            Self::ConfirmWork => ManualClassification::Credited,
            Self::Exclude => ManualClassification::Excluded,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::ConfirmWork => "confirm-work",
            Self::Exclude => "exclude",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LongLockReviewAdjustmentCommand {
    pub adjustment_id: AdjustmentId,
    pub review_id: LongLockReviewId,
    pub locked_episode: UtcInterval,
    /// Used only by the first claim; a retry keeps the creation instant that claim stored.
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishLongLockReviewCommand {
    pub review_id: LongLockReviewId,
    pub locked_episode: UtcInterval,
}

#[derive(Debug)]
pub enum ReviewError {
    NotPending(LongLockReviewId),
    /// The review's locked episode is no longer the one the command names.
    LockedEpisodeChanged(LongLockReviewId),
    /// The command's episode lies outside the accounted part of the day; .NET reports it as
    /// `LockedEpisodeChanged`.
    EpisodeOutsideHorizon(LongLockReviewId),
    /// The lock has not ended yet.
    StillOpen(LongLockReviewId),
    /// The review is of a recurring appointment, which the day goes on after.
    NotALock(LongLockReviewId),
    OutsideCurrentWeek,
    /// Includes [`LongLockResolutionError::Conflict`] when another action resolved the review.
    Resolutions(LongLockResolutionError),
    Adjustments(ManualAdjustmentError),
    Events(SessionEventError),
    Accounting(AccountingError),
}

impl std::fmt::Display for ReviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotPending(id) => write!(f, "Long-lock review '{id}' is not pending."),
            Self::LockedEpisodeChanged(id) => write!(
                f,
                "Long-lock review '{id}' no longer has the requested locked episode."
            ),
            Self::EpisodeOutsideHorizon(id) => write!(
                f,
                "Long-lock review '{id}' has no portion in the current accounting horizon."
            ),
            Self::StillOpen(id) => write!(
                f,
                "Long-lock review '{id}' is still open and cannot be resolved yet."
            ),
            Self::NotALock(id) => write!(
                f,
                "Review '{id}' is of a recurring appointment; the day cannot finish at it."
            ),
            Self::OutsideCurrentWeek => f.write_str(
                "A long-lock review can only be resolved for a day of the current local week.",
            ),
            Self::Resolutions(error) => error.fmt(f),
            Self::Adjustments(error) => error.fmt(f),
            Self::Events(error) => error.fmt(f),
            Self::Accounting(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ReviewError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Resolutions(error) => Some(error),
            Self::Adjustments(error) => Some(error),
            Self::Events(error) => Some(error),
            Self::Accounting(error) => Some(error),
            _ => None,
        }
    }
}

impl From<LongLockResolutionError> for ReviewError {
    fn from(error: LongLockResolutionError) -> Self {
        Self::Resolutions(error)
    }
}

impl From<ManualAdjustmentError> for ReviewError {
    fn from(error: ManualAdjustmentError) -> Self {
        Self::Adjustments(error)
    }
}

impl From<SessionEventError> for ReviewError {
    fn from(error: SessionEventError) -> Self {
        Self::Events(error)
    }
}

impl From<AccountingError> for ReviewError {
    fn from(error: AccountingError) -> Self {
        Self::Accounting(error)
    }
}

pub struct LongLockReviews<'a> {
    adjustments: ManualAdjustmentRepository<'a>,
    events: SessionEventRepository<'a>,
    resolutions: LongLockResolutionRepository<'a>,
}

impl<'a> LongLockReviews<'a> {
    pub fn new(
        adjustments: ManualAdjustmentRepository<'a>,
        events: SessionEventRepository<'a>,
        resolutions: LongLockResolutionRepository<'a>,
    ) -> Self {
        Self {
            adjustments,
            events,
            resolutions,
        }
    }

    /// Resolves review `review_id` of `workday`, a day of the current local week, as the day is
    /// accounted now. Once the review is resolved, the same resolution again finds its adjustment
    /// already recorded.
    pub fn resolve(
        &self,
        accounting: &WorkdayAccounting,
        workday: &LocalWorkday,
        review_id: &LongLockReviewId,
        resolution: ManualResolution,
    ) -> Result<AppendAdjustmentResult, ReviewError> {
        let context = accounting.capture_context();
        let today = context.calendar().workday(context.observed_at).date;
        if workday.date < start_of_week(today) || workday.date > today {
            return Err(ReviewError::OutsideCurrentWeek);
        }

        let calculation = accounting.calculate(workday.date, &context)?;
        let current = calculation.summary_input.accounting();
        let adjustment_id = adjustment_id_for(workday, review_id, resolution);
        let review = if current.timeline().window().workday == *workday {
            pending(current, review_id)
        } else {
            None
        };
        let Some(review) = review else {
            return if workday.is_working_day()
                && self
                    .adjustments
                    .read_for_workday(workday)?
                    .iter()
                    .any(|adjustment| *adjustment.id() == adjustment_id)
            {
                Ok(AppendAdjustmentResult {
                    outcome: AppendAdjustmentOutcome::AlreadyRecorded,
                    authoritative_id: adjustment_id,
                })
            } else {
                Err(ReviewError::NotPending(review_id.clone()))
            };
        };
        let command = LongLockReviewAdjustmentCommand {
            adjustment_id,
            review_id: review.id.clone(),
            locked_episode: review.locked_episode,
            created_at: context.observed_at,
        };
        self.append_resolution(current, &command, resolution)
    }

    /// Credits the locked episode as work.
    pub fn confirm_as_work(
        &self,
        current: &AccountingResult,
        command: &LongLockReviewAdjustmentCommand,
    ) -> Result<AppendAdjustmentResult, ReviewError> {
        self.append_resolution(current, command, ManualResolution::ConfirmWork)
    }

    /// Excludes the locked episode from work.
    pub fn reclassify_as_not_work(
        &self,
        current: &AccountingResult,
        command: &LongLockReviewAdjustmentCommand,
    ) -> Result<AppendAdjustmentResult, ReviewError> {
        self.append_resolution(current, command, ManualResolution::Exclude)
    }

    /// Finishes the day at the start of the locked episode, so that nothing after it counts.
    pub fn finish_at_lock_start(
        &self,
        current: &AccountingResult,
        command: &FinishLongLockReviewCommand,
    ) -> Result<RecordTransitionResult, ReviewError> {
        let window = current.timeline().window();
        match pending(current, &command.review_id) {
            Some(review) if review.reason == LongLockReviewReason::RecurringAppointment => {
                return Err(ReviewError::NotALock(review.id.clone()));
            }
            Some(review) => validate(review, command.locked_episode)?,
            None if self.was_finished_at(window, command.locked_episode.start())? => {}
            None => return Err(ReviewError::NotPending(command.review_id.clone())),
        }

        let time_zone_id = &window.workday.time_zone_id;
        let operation_key = OperationKey::new(format!(
            "{FINISH_OPERATION_KEY_PREFIX}:{}:finish-at-lock-start",
            command.review_id
        ))
        .expect("the key starts with its prefix");
        let identity = [
            operation_key.to_string(),
            format_instant(command.locked_episode.start()),
            format_instant(command.locked_episode.end()),
            time_zone_id.clone(),
            "Finished".to_owned(),
        ]
        .join(IDENTITY_SEPARATOR);
        let claim = LongLockResolutionClaim {
            workday: window.workday.clone(),
            review_id: command.review_id.clone(),
            action: LongLockResolutionAction::new(
                LongLockResolutionActionKind::FinishAtLockStart,
                &identity,
            )
            .expect("the identity starts with the operation key"),
            proposed_created_at: command.locked_episode.start(),
        };
        self.resolutions.claim(&claim)?;
        let transition = WorkdayTransition::new(
            command.locked_episode.start(),
            time_zone_id,
            WorkdayTransitionKind::TrackingChanged(TrackingState::Finished),
        )
        .expect("a workday's time zone is named");
        match self.events.record(operation_key, transition) {
            Err(error @ SessionEventError::OperationConflict(_)) => {
                self.resolutions.release(&claim)?;
                Err(error.into())
            }
            result => Ok(result?),
        }
    }

    fn append_resolution(
        &self,
        current: &AccountingResult,
        command: &LongLockReviewAdjustmentCommand,
        resolution: ManualResolution,
    ) -> Result<AppendAdjustmentResult, ReviewError> {
        let review_id = &command.review_id;
        let pending = pending(current, review_id);
        if let Some(review) = pending {
            validate(review, command.locked_episode)?;
        }

        let timeline = current.timeline();
        let workday = &timeline.window().workday;
        if timeline.through() <= timeline.window().start || !workday.is_working_day() {
            return Err(ReviewError::NotPending(review_id.clone()));
        }
        let interval = resolution_interval(current, command.locked_episode)
            .ok_or_else(|| ReviewError::EpisodeOutsideHorizon(review_id.clone()))?;

        let identity = [
            command.adjustment_id.as_str(),
            &format_instant(interval.start()),
            &format_instant(interval.end()),
            match resolution.classification() {
                ManualClassification::Credited => "Credited",
                ManualClassification::Excluded => "Excluded",
            },
        ]
        .join(IDENTITY_SEPARATOR);
        let claim = LongLockResolutionClaim {
            workday: workday.clone(),
            review_id: review_id.clone(),
            action: LongLockResolutionAction::new(resolution.action_kind(), &identity)
                .expect("the identity starts with the adjustment id"),
            proposed_created_at: command.created_at,
        };
        let claimed = self.resolutions.claim(&claim)?;
        let candidate = match resolution {
            ManualResolution::ConfirmWork => ManualAdjustment::long_lock_confirmation(
                command.adjustment_id.clone(),
                workday.clone(),
                interval,
                claimed.canonical_created_at,
                review_id.clone(),
            ),
            ManualResolution::Exclude => ManualAdjustment::reclassification(
                command.adjustment_id.clone(),
                workday.clone(),
                interval,
                ManualClassification::Excluded,
                claimed.canonical_created_at,
            ),
        }
        .expect("a resolution of a working day's review is a valid adjustment");

        let exact_retry = self
            .adjustments
            .read_for_workday(workday)?
            .contains(&candidate);
        if exact_retry {
            return Ok(AppendAdjustmentResult {
                outcome: AppendAdjustmentOutcome::AlreadyRecorded,
                authoritative_id: candidate.id().clone(),
            });
        }
        // Another caller of this action shares the claim and may record the resolution at any
        // moment: the claim is released only while the resolution is unrecorded, and recorded
        // only while the claim is held.
        if pending.is_none() {
            self.adjustments.release_unrecorded(&claim, &candidate)?;
            return Err(ReviewError::NotPending(review_id.clone()));
        }

        match self.adjustments.append_claimed(current, &candidate, &claim) {
            Err(ManualAdjustmentError::ClaimReleased(_)) => {
                Err(ReviewError::NotPending(review_id.clone()))
            }
            Err(error) if is_deterministic(&error) => {
                self.adjustments.release_unrecorded(&claim, &candidate)?;
                Err(error.into())
            }
            result => Ok(result?),
        }
    }

    fn was_finished_at(
        &self,
        window: &WorkdayWindow,
        lock_start: Timestamp,
    ) -> Result<bool, ReviewError> {
        let history = self.events.read_for_accounting(window)?;
        let before_window = history
            .before_window()
            .tracking
            .as_ref()
            .is_some_and(|anchor| {
                anchor.state == TrackingState::Finished && anchor.occurred_at == lock_start
            });
        let in_window = history.in_window().iter().any(|transition| {
            matches!(
                transition,
                RecordedTransition::Tracking(recorded)
                    if recorded.state == TrackingState::Finished
                        && recorded.occurred_at == lock_start
            )
        });
        Ok(before_window || in_window)
    }
}

/// The id of the adjustment that records `resolution` of review `review_id`: the same for every
/// retry, and distinct per workday, zone, review and action.
pub fn adjustment_id_for(
    workday: &LocalWorkday,
    review_id: &LongLockReviewId,
    resolution: ManualResolution,
) -> AdjustmentId {
    AdjustmentId::new(format!(
        "{RESOLUTION_ADJUSTMENT_ID_PREFIX}{}:{}:{review_id}:{}",
        format_date(workday.date),
        base64_url(workday.time_zone_id.as_bytes()),
        resolution.name()
    ))
    .expect("the id starts with its prefix")
}

/// The review an adjustment of [`adjustment_id_for`] resolves, and how; `None` for any other.
pub fn resolution_of(id: &AdjustmentId) -> Option<(LongLockReviewId, ManualResolution)> {
    let rest = id.as_str().strip_prefix(RESOLUTION_ADJUSTMENT_ID_PREFIX)?;
    let (_date, rest) = rest.split_once(':')?;
    let (_zone, rest) = rest.split_once(':')?;
    let (review, name) = rest.rsplit_once(':')?;
    let resolution = [ManualResolution::ConfirmWork, ManualResolution::Exclude]
        .into_iter()
        .find(|resolution| resolution.name() == name)?;
    Some((LongLockReviewId::new(review).ok()?, resolution))
}

/// Whether the adjustment records a review resolution, which stays bound to its claim and so
/// cannot be withdrawn.
pub fn is_resolution_adjustment(id: &AdjustmentId) -> bool {
    id.as_str().starts_with(RESOLUTION_ADJUSTMENT_ID_PREFIX)
}

/// What confirming or excluding the lock `locked_episode` records on the day `current`
/// accounts: the whole lock within the accounted part of the day, lunch and other exclusions
/// included. `None` when no part of the lock falls there.
pub fn resolution_interval(
    current: &AccountingResult,
    locked_episode: UtcInterval,
) -> Option<UtcInterval> {
    let timeline = current.timeline();
    let horizon = UtcInterval::new(timeline.window().start, timeline.through()).ok()?;
    let interval = locked_episode.intersect(horizon)?;
    UtcInterval::new(
        truncate_to_tick(interval.start()),
        truncate_to_tick(interval.end()),
    )
    .ok()
}

fn pending<'r>(
    current: &'r AccountingResult,
    review_id: &LongLockReviewId,
) -> Option<&'r LongLockReview> {
    current
        .pending_reviews()
        .iter()
        .find(|review| review.id == *review_id)
}

fn validate(review: &LongLockReview, locked_episode: UtcInterval) -> Result<(), ReviewError> {
    if review.locked_episode != locked_episode {
        return Err(ReviewError::LockedEpisodeChanged(review.id.clone()));
    }
    if review.is_open {
        return Err(ReviewError::StillOpen(review.id.clone()));
    }
    Ok(())
}

/// A failure that the same append would meet again, so the claim must not keep the review.
fn is_deterministic(error: &ManualAdjustmentError) -> bool {
    match error {
        ManualAdjustmentError::Validation(_)
        | ManualAdjustmentError::IdConflict(_)
        | ManualAdjustmentError::ParentConflict { .. }
        | ManualAdjustmentError::SupersessionConflict { .. } => true,
        ManualAdjustmentError::Storage(_)
        | ManualAdjustmentError::FractionalMinutes(_)
        | ManualAdjustmentError::IntervalShorterThanATick(_)
        | ManualAdjustmentError::NotAWorkingDay
        | ManualAdjustmentError::BlankTimeZone
        | ManualAdjustmentError::ClaimReleased(_) => false,
    }
}

/// Unpadded base64url, as .NET's `Convert.ToBase64String` with `+` and `/` replaced and `=`
/// trimmed.
pub fn base64_url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = chunk
            .iter()
            .enumerate()
            .fold(0u32, |group, (index, &byte)| {
                group | u32::from(byte) << (16 - 8 * index)
            });
        for index in 0..=chunk.len() {
            encoded.push(char::from(
                ALPHABET[(group >> (18 - 6 * index) & 0x3F) as usize],
            ));
        }
    }
    encoded
}

#[cfg(test)]
mod tests;
