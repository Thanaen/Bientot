//! The week read from a ledger and the week's corrections written to it, shared by the live and
//! demo hosts. Ported from the commands of `HistoryWorkspace.cs`: each one checks that its day
//! still shows what the user answered about, and a refusal names the message the week shows.

use std::collections::{HashMap, HashSet};

use jiff::SignedDuration;
use rusqlite::Connection;
use worktime_core::accounting::{CalculationContext, WorkdayAccounting};
use worktime_core::corrections::{
    Binding, CorrectionError, ManualAdjustmentCommands, binding, history_by_id,
};
use worktime_core::history::HistoryService;
use worktime_core::ranges::{RangeCommand, RangeCorrections, RangeError};
use worktime_core::reviews::{
    FinishLongLockReviewCommand, LongLockReviews, ReviewError, resolution_interval,
};
use worktime_domain::accounting::{AccountingLabel, AdjustmentId};
use worktime_domain::adjustments::{
    AdjustmentValidationError, ManualAdjustment, ManualClassification,
};
use worktime_domain::configuration::{AppointmentProposal, ObservedDay, WorkTimeSettings};
use worktime_domain::history::DailySummaryDisposition;
use worktime_domain::history::{CurrentWeek, DailySummarySnapshot};
use worktime_domain::intervals::UtcInterval;
use worktime_domain::time::{Clock, LocalWorkday, TimeZoneProvider};
use worktime_store::adjustments::{ManualAdjustmentError, ManualAdjustmentRepository};
use worktime_store::history::DailySummaryRepository;
use worktime_store::reviews::{LongLockResolutionError, LongLockResolutionRepository};
use worktime_store::sessions::SessionEventRepository;

use crate::strings::Key;
use crate::week::{Gains, LockReview, Offer, WeekCommand, WeekTimeline};

/// The current week as the ledger stores it, placed on its timeline, with the locks it awaits
/// an answer for as `accounting` resolves them and what it offers on each correction.
pub fn read_week(
    connection: &Connection,
    accounting: &WorkdayAccounting,
    clock: &dyn Clock,
    zones: &dyn TimeZoneProvider,
) -> anyhow::Result<WeekTimeline> {
    let week = HistoryService::new(DailySummaryRepository::new(connection), clock, zones)
        .current_week()?;
    let reviews = pending_locks(&week, connection, accounting, clock);
    let (offers, confirmed): (HashMap<_, _>, Vec<_>) = offers(connection, &week);
    Ok(WeekTimeline::project(
        &week,
        &reviews,
        &offers,
        &confirmed,
        clock.now(),
        zones,
    ))
}

/// « Compter chaque jeudi… » after a correction of `workday`: the appointment the stored days
/// of its weekday suggest, unless the user turned proposals off. A day that cannot be read
/// counts as a day off.
pub fn proposal(
    workday: &LocalWorkday,
    settings: &WorkTimeSettings,
    connection: &Connection,
    zones: &dyn TimeZoneProvider,
) -> Option<AppointmentProposal> {
    if !settings.appointment_proposals.enabled {
        return None;
    }
    let history = DailySummaryRepository::new(connection);
    let days: Vec<ObservedDay> = (0..AppointmentProposal::LOOKBACK_DAYS / 7)
        .filter_map(|week| workday.date.checked_sub(jiff::Span::new().weeks(week)).ok())
        .filter_map(|date| history.read_latest(date).ok().flatten())
        .filter_map(|revision| {
            let snapshot = revision.snapshot();
            let zone = zones.get(&snapshot.workday().time_zone_id).ok()?;
            let local = |instant: jiff::Timestamp| instant.to_zoned(zone.clone()).time();
            let entries = snapshot.breakdown().entries();
            Some(ObservedDay {
                date: snapshot.workday().date,
                worked: entries
                    .iter()
                    .any(|entry| entry.label() == AccountingLabel::ActiveWork),
                corrected: entries
                    .iter()
                    .filter(|entry| {
                        entry.disposition() == DailySummaryDisposition::ConfirmedCredit
                            && entry.adjustment_id().is_some()
                    })
                    .filter_map(|entry| entry.interval())
                    .map(|interval| (local(interval.start()), local(interval.end())))
                    .collect(),
            })
        })
        .collect();
    AppointmentProposal::find(
        workday.date.weekday(),
        &days,
        workday.date,
        &settings.recurring_appointments,
        &settings.appointment_proposals.dismissed,
    )
}

/// The locks confirmed as work, each with the correction that confirms it.
type Confirmed = Vec<(AdjustmentId, UtcInterval)>;

/// What the week offers on each correction its summaries count, from the history of its day,
/// and the locks each day confirmed as work.
fn offers(
    connection: &Connection,
    week: &CurrentWeek,
) -> (HashMap<AdjustmentId, Offer>, Confirmed) {
    let (offers, confirmed): (Vec<_>, Vec<_>) = week
        .days()
        .iter()
        .filter_map(|day| day.latest_revision.as_ref())
        .map(|revision| day_offers(connection, revision.snapshot()))
        .unzip();
    (offers.into_iter().flatten().collect(), confirmed.concat())
}

/// What the week offers on each correction `snapshot` counts. A day whose history cannot be
/// read names none of them, which the week takes as [`Offer::Unknown`].
fn day_offers(
    connection: &Connection,
    snapshot: &DailySummarySnapshot,
) -> (Vec<(AdjustmentId, Offer)>, Confirmed) {
    let workday = snapshot.workday();
    let history = match ManualAdjustmentRepository::new(connection).read_for_workday(workday) {
        Ok(history) => history_by_id(history),
        Err(error) => {
            log::warn!("cannot read the corrections of {}: {error}", workday.date);
            return (Vec::new(), Vec::new());
        }
    };
    let replaced: HashSet<&AdjustmentId> = history
        .values()
        .filter_map(ManualAdjustment::supersedes_id)
        .collect();
    // Every lock still confirmed as work, though the summary may name none of its pieces when
    // it slept all along. An appointment's absence counts asleep or not.
    let confirmed: Confirmed = history
        .values()
        .filter(|adjustment| !replaced.contains(adjustment.id()))
        .filter(|adjustment| {
            adjustment
                .kind()
                .confirmed_review()
                .is_some_and(|review| review.appointment_id().is_none())
        })
        .filter_map(|adjustment| Some((adjustment.id().clone(), adjustment.interval()?)))
        .collect();
    let offers = snapshot
        .breakdown()
        .entries()
        .iter()
        .filter_map(|entry| entry.adjustment_id())
        .chain(confirmed.iter().map(|(id, _)| id))
        .collect::<HashSet<_>>()
        .into_iter()
        .filter_map(|id| Some((id.clone(), offer(history.get(id)?, &history))))
        .collect();
    (offers, confirmed)
}

/// What the week offers on `adjustment`, as its workday's `history` binds it to a review:
/// « Corriger le temps… » replaces any correction but a long-lock confirmation, and the ledger
/// withdraws only one bound to no review.
fn offer(
    adjustment: &ManualAdjustment,
    history: &HashMap<AdjustmentId, ManualAdjustment>,
) -> Offer {
    match binding(adjustment, history) {
        Binding::Free => Offer::EditAndRemove,
        Binding::FromResolution => Offer::EditOnly,
        Binding::ConfirmsReview => Offer::Locked,
        // A revocation counts nothing: no summary names one.
        Binding::Revocation => Offer::Unknown,
    }
}

/// The locks of `week` that await an answer, each as a review command resolves it: a stored
/// summary keeps neither a lock's bounds nor whether it ended. A day that cannot be
/// recalculated offers none, as its review commands would fail the same way.
fn pending_locks(
    week: &CurrentWeek,
    connection: &Connection,
    accounting: &WorkdayAccounting,
    clock: &dyn Clock,
) -> Vec<LockReview> {
    let context = accounting.capture_context();
    let mut locks = Vec::new();
    let snapshots = week
        .days()
        .iter()
        .filter_map(|day| day.latest_revision.as_ref())
        .map(|revision| revision.snapshot())
        .filter(|snapshot| snapshot.has_pending_review());
    for snapshot in snapshots {
        let workday = snapshot.workday();
        let calculation = match accounting.calculate(workday.date, &context) {
            Ok(calculation) => calculation,
            Err(error) => {
                log::warn!("cannot read the long locks of {}: {error}", workday.date);
                continue;
            }
        };
        let current = calculation.summary_input.accounting();
        if current.timeline().window().workday != *workday {
            continue;
        }
        let reviews: Vec<_> = current
            .pending_reviews()
            .iter()
            .filter_map(|review| {
                Some((review, resolution_interval(current, review.locked_episode)?))
            })
            .collect();
        locks.extend(reviews.into_iter().map(|(review, resolved)| {
            let gain = |classification| {
                (!review.is_open)
                    .then(|| {
                        answered_credit(
                            connection,
                            accounting,
                            &context,
                            clock,
                            workday,
                            resolved,
                            classification,
                        )
                    })
                    .flatten()
                    .map(|after| after - current.credited_work())
            };
            LockReview {
                id: review.id.clone(),
                locked_episode: review.locked_episode,
                resolved,
                is_open: review.is_open,
                appointment: review.appointment_name(&context.settings.recurring_appointments),
                weekday: workday.date.weekday(),
                gains: gain(ManualClassification::Credited)
                    .zip(gain(ManualClassification::Excluded))
                    .map(|(count, exclude)| Gains { count, exclude }),
            }
        }));
    }
    locks
}

/// The day's credit once `range` of `workday` is answered as `classification`, as `context`
/// observes it: the range is recorded in a transaction rolled back once the day is calculated
/// again. The same observation on both sides keeps the work tracked meanwhile out of the gain.
fn answered_credit(
    connection: &Connection,
    accounting: &WorkdayAccounting,
    context: &CalculationContext,
    clock: &dyn Clock,
    workday: &LocalWorkday,
    range: UtcInterval,
    classification: ManualClassification,
) -> Option<SignedDuration> {
    let rolled_back = ManualAdjustmentRepository::new(connection).scope().ok()?;
    RangeCorrections::new(
        accounting,
        ManualAdjustmentRepository::new(connection),
        LongLockReviews::new(
            ManualAdjustmentRepository::new(connection),
            SessionEventRepository::new(connection, clock),
            LongLockResolutionRepository::new(connection),
        ),
    )
    .record(&RangeCommand {
        workday: workday.clone(),
        range,
        classification,
        id: AdjustmentId::new("preview").expect("the id is not blank"),
        created_at: clock.now(),
    })
    .ok()?;
    let calculation = accounting.calculate(workday.date, context).ok()?;
    drop(rolled_back);
    Some(calculation.summary_input.accounting().credited_work())
}

/// Why a command was not recorded: its message, and the error to log when there is one.
pub struct Refusal {
    pub message: Key,
    pub error: Option<String>,
}

impl Refusal {
    fn changed() -> Self {
        Self {
            message: Key::HistoryPeriodResolved,
            error: None,
        }
    }

    fn failed(message: Key, error: impl std::fmt::Display) -> Self {
        Self {
            message,
            error: Some(error.to_string()),
        }
    }

    /// The message to show, once `log` wrote the error, when there is one.
    pub fn logged(self, log: impl FnOnce(&String)) -> Key {
        if let Some(error) = &self.error {
            log(error);
        }
        self.message
    }
}

/// Records `command`. A range returns the corrections whose withdrawal undoes it, for
/// « Annuler »: none once it replaced a correction or settled a lock.
pub fn run(
    command: &WeekCommand,
    connection: &Connection,
    accounting: &WorkdayAccounting,
    clock: &dyn Clock,
) -> Result<Vec<AdjustmentId>, Refusal> {
    let corrections =
        ManualAdjustmentCommands::new(accounting, ManualAdjustmentRepository::new(connection));
    let reviews = || {
        LongLockReviews::new(
            ManualAdjustmentRepository::new(connection),
            SessionEventRepository::new(connection, clock),
            LongLockResolutionRepository::new(connection),
        )
    };
    let new_id =
        || AdjustmentId::new(format!("{:032x}", crate::random_id())).expect("the id is not blank");
    match command {
        WeekCommand::Range {
            workday,
            range,
            classification,
        } => RangeCorrections::new(
            accounting,
            ManualAdjustmentRepository::new(connection),
            reviews(),
        )
        .record(&RangeCommand {
            workday: workday.clone(),
            range: *range,
            classification: *classification,
            id: new_id(),
            created_at: clock.now(),
        })
        .map(|outcome| outcome.undoable().map(<[_]>::to_vec).unwrap_or_default())
        .map_err(range_refusal),
        WeekCommand::FinishAtLockStart { workday, review } => {
            let context = accounting.capture_context();
            let calculation = accounting
                .calculate(workday.date, &context)
                .map_err(|error| Refusal::failed(Key::HistoryCorrectionFailed, error))?;
            let current = calculation.summary_input.accounting();
            let pending = current
                .pending_reviews()
                .iter()
                .find(|pending| pending.id == *review && !pending.is_open)
                .ok_or_else(Refusal::changed)?;
            reviews()
                .finish_at_lock_start(
                    current,
                    &FinishLongLockReviewCommand {
                        review_id: review.clone(),
                        locked_episode: pending.locked_episode,
                    },
                )
                .map(|_| Vec::new())
                .map_err(review_refusal)
        }
        WeekCommand::Revoke {
            workday,
            adjustments,
        } => {
            // Undo and remove race with edits elsewhere: only active corrections are withdrawn.
            let mut revocations = Vec::new();
            for adjustment in adjustments {
                let active = corrections
                    .active(workday, adjustment)
                    .map_err(|error| Refusal::failed(Key::HistoryCorrectionFailed, error))?;
                if active.is_none() {
                    return Err(Refusal::changed());
                }
                revocations.push(
                    ManualAdjustment::revocation(
                        new_id(),
                        workday.clone(),
                        adjustment.clone(),
                        clock.now(),
                    )
                    .map_err(|error| Refusal::failed(Key::HistoryCorrectionFailed, error))?,
                );
            }
            corrections
                .save_all(&revocations)
                .map(|_| Vec::new())
                .map_err(correction_refusal)
        }
    }
}

/// The correction `id` of `workday`, unless a later one replaced it, as `EditAsync` of
/// `HistoryWorkspace` reads it before opening « Corriger le temps… » on it.
pub fn active_adjustment(
    workday: &LocalWorkday,
    id: &AdjustmentId,
    connection: &Connection,
    accounting: &WorkdayAccounting,
) -> Result<Option<ManualAdjustment>, Refusal> {
    ManualAdjustmentCommands::new(accounting, ManualAdjustmentRepository::new(connection))
        .active(workday, id)
        .map_err(|error| Refusal::failed(Key::HistoryCorrectionFailed, error))
}

/// Records a correction of « Corriger le temps… », refused with the message the window shows,
/// as `AdjustTimeSubmissionErrorMapper` maps it.
pub fn save(
    adjustment: &ManualAdjustment,
    connection: &Connection,
    accounting: &WorkdayAccounting,
) -> Result<AdjustmentId, Refusal> {
    use AdjustmentValidationError::{
        ConflictingId, OutsideAccountingHorizon, OverlappingActiveAdjustments,
        OverlapsCreditedCoverage,
    };
    ManualAdjustmentCommands::new(accounting, ManualAdjustmentRepository::new(connection))
        .save(adjustment)
        .map(|saved| saved.authoritative_id)
        .map_err(|error| {
            let message = match &error {
                CorrectionError::Adjustments(ManualAdjustmentError::Validation(validation)) => {
                    match validation {
                        OverlapsCreditedCoverage { .. } => Key::AdjustTimeErrorOverlapCredited,
                        OverlappingActiveAdjustments => Key::AdjustTimeErrorActiveOverlap,
                        OutsideAccountingHorizon { .. } => Key::AdjustTimeErrorOutsideHorizon,
                        ConflictingId { .. } => Key::AdjustTimeErrorConflict,
                        _ => Key::AdjustTimeErrorSubmissionFailed,
                    }
                }
                CorrectionError::Adjustments(
                    ManualAdjustmentError::IdConflict(_)
                    | ManualAdjustmentError::ParentConflict { .. }
                    | ManualAdjustmentError::SupersessionConflict { .. },
                ) => Key::AdjustTimeErrorConflict,
                CorrectionError::OutsideCurrentWeek => Key::AdjustTimeErrorOutsideWeek,
                _ => Key::AdjustTimeErrorSubmissionFailed,
            };
            Refusal::failed(message, error)
        })
}

fn correction_refusal(error: CorrectionError) -> Refusal {
    match error {
        CorrectionError::OutsideCurrentWeek => Refusal::failed(Key::HistoryWeekChanged, error),
        CorrectionError::Adjustments(ManualAdjustmentError::Validation(
            AdjustmentValidationError::InvalidRevocationTarget { .. },
        )) => Refusal {
            message: Key::HistoryCorrectionNotRemovable,
            error: None,
        },
        error => Refusal::failed(Key::HistoryCorrectionFailed, error),
    }
}

fn range_refusal(error: RangeError) -> Refusal {
    match error {
        RangeError::OutsideCurrentWeek => Refusal::failed(Key::HistoryWeekChanged, error),
        RangeError::Bound(_) => Refusal {
            message: Key::HistoryRangeBound,
            error: None,
        },
        RangeError::Review(error) => review_refusal(error),
        RangeError::Adjustments(
            ManualAdjustmentError::IdConflict(_)
            | ManualAdjustmentError::ParentConflict { .. }
            | ManualAdjustmentError::SupersessionConflict { .. },
        ) => Refusal::changed(),
        error => Refusal::failed(Key::HistoryCorrectionFailed, error),
    }
}

/// `LongLockReviewResolutionException` of the .NET app: the review moved on since it was shown.
fn review_refusal(error: ReviewError) -> Refusal {
    match error {
        ReviewError::OutsideCurrentWeek => Refusal::failed(Key::HistoryWeekChanged, error),
        ReviewError::NotPending(_)
        | ReviewError::LockedEpisodeChanged(_)
        | ReviewError::EpisodeOutsideHorizon(_)
        | ReviewError::StillOpen(_)
        | ReviewError::Resolutions(LongLockResolutionError::Conflict(_)) => Refusal::changed(),
        error => Refusal::failed(Key::HistoryCorrectionFailed, error),
    }
}
