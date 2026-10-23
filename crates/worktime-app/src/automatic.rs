//! « Compter automatiquement » (issue #321, lot 3): an absence in a recurring appointment
//! counted without asking is recorded as a correction of its own once the user is back, which
//! the flyout announces with « Ce n’était pas du travail ». After a few « Oui, je travaillais »
//! in a row, the flyout offers to count the appointment so from then on.

use std::collections::HashSet;

use jiff::{SignedDuration, Timestamp, ToSpan as _};
use rusqlite::Connection;
use worktime_core::reviews::{ManualResolution, base64_url, resolution_interval, resolution_of};
use worktime_domain::accounting::{
    AccountingResult, AdjustmentId, LongLockReviewId, LongLockReviewReason,
};
use worktime_domain::adjustments::{ManualAdjustment, ManualClassification};
use worktime_domain::configuration::RecurringAppointment;
use worktime_domain::intervals::UtcInterval;
use worktime_domain::time::LocalWorkday;
use worktime_store::adjustments::ManualAdjustmentRepository;
use worktime_store::history::DailySummaryRepository;

const COUNT_PREFIX: &str = "appointment-auto:";
const UNDO_SUFFIX: &str = ":not-work";

/// The « Oui, je travaillais » in a row after which the flyout offers to count automatically.
pub const TRUST_STREAK: usize = 4;

/// An absence counted automatically, as the flyout announces it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountedAbsence {
    pub workday: LocalWorkday,
    pub adjustment_id: AdjustmentId,
    pub interval: UtcInterval,
    pub local_start: jiff::civil::Time,
    pub local_end: jiff::civil::Time,
    pub duration: SignedDuration,
    /// The name the user gave the appointment, empty for none.
    pub appointment: String,
}

/// « Compter automatiquement à l’avenir », after [`TRUST_STREAK`] confirmations in a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustOffer {
    pub appointment_id: String,
    /// The name the user gave the appointment, empty for none.
    pub appointment: String,
    pub weekday: jiff::civil::Weekday,
    pub streak: usize,
}

/// `appointment-auto:2026-10-22:<zone>:<review>`: one count per absence and zone of its day,
/// which a retry finds.
pub fn count_id(workday: &LocalWorkday, review: &LongLockReviewId) -> AdjustmentId {
    AdjustmentId::new(format!(
        "{COUNT_PREFIX}{}:{}:{review}",
        workday.date,
        base64_url(workday.time_zone_id.as_bytes())
    ))
    .expect("the id starts with its prefix")
}

/// The absence a count of [`count_id`] records.
pub fn counted_review(id: &AdjustmentId) -> Option<LongLockReviewId> {
    let rest = id.as_str().strip_prefix(COUNT_PREFIX)?;
    let (_date, rest) = rest.split_once(':')?;
    let (_zone, review) = rest.split_once(':')?;
    LongLockReviewId::new(review.strip_suffix(UNDO_SUFFIX).unwrap_or(review)).ok()
}

fn appointment<'a>(
    review: &LongLockReviewId,
    appointments: &'a [RecurringAppointment],
) -> Option<&'a RecurringAppointment> {
    let id = review.appointment_id()?;
    appointments.iter().find(|appointment| appointment.id == id)
}

/// The counts to record on `workday`, whose accounting is `current` at `now`: one per absence
/// in an automatic appointment that ended, the user being back, unless one was recorded before,
/// even if withdrawn since.
pub fn counts(
    workday: &LocalWorkday,
    current: &AccountingResult,
    now: Timestamp,
    appointments: &[RecurringAppointment],
    recorded: &[ManualAdjustment],
) -> Vec<ManualAdjustment> {
    let known: HashSet<&AdjustmentId> = recorded.iter().map(ManualAdjustment::id).collect();
    current
        .pending_reviews()
        .iter()
        .filter(|review| {
            !review.is_open
                && review.reason == LongLockReviewReason::RecurringAppointment
                && appointment(&review.id, appointments).is_some_and(|found| found.automatic)
        })
        .filter_map(|review| {
            let id = count_id(workday, &review.id);
            if known.contains(&id) {
                return None;
            }
            let interval = resolution_interval(current, review.locked_episode)?;
            ManualAdjustment::reclassification(
                id,
                workday.clone(),
                interval,
                ManualClassification::Credited,
                now,
            )
            .ok()
        })
        .collect()
}

/// The counts of `adjustments` still in effect, which no later correction replaced.
pub fn in_effect(adjustments: &[ManualAdjustment]) -> Vec<&ManualAdjustment> {
    let replaced: HashSet<&AdjustmentId> = adjustments
        .iter()
        .filter_map(ManualAdjustment::supersedes_id)
        .collect();
    adjustments
        .iter()
        .filter(|adjustment| {
            adjustment.id().as_str().starts_with(COUNT_PREFIX)
                && !adjustment.id().as_str().ends_with(UNDO_SUFFIX)
                && !replaced.contains(adjustment.id())
        })
        .collect()
}

/// « Ce n’était pas du travail » on `count`: the same time excluded in its place, created
/// after it even within the same instant.
pub fn undo(count: &ManualAdjustment, now: Timestamp) -> Option<ManualAdjustment> {
    let interval = count.interval()?;
    let now = now.max(count.created_at() + SignedDuration::from_micros(1));
    ManualAdjustment::reclassification(
        AdjustmentId::new(format!("{}{UNDO_SUFFIX}", count.id())).ok()?,
        count.workday().clone(),
        interval,
        ManualClassification::Excluded,
        now,
    )
    .ok()
    .map(|exclusion| exclusion.superseding(count.id().clone()))
}

/// How the user answered about the absences of `appointment_id` on `adjustments` of one day:
/// `Some(true)` for work, `Some(false)` for not work, `None` without an answer.
fn answer(appointment_id: &str, adjustments: &[ManualAdjustment]) -> Option<bool> {
    let ours = |review: &LongLockReviewId| review.appointment_id() == Some(appointment_id);
    let mut answer = None;
    for adjustment in adjustments {
        if let Some((review, resolution)) = resolution_of(adjustment.id())
            && ours(&review)
        {
            let work = resolution == ManualResolution::ConfirmWork;
            answer = Some(answer.unwrap_or(true) && work);
        }
    }
    for count in in_effect(adjustments) {
        if counted_review(count.id()).is_some_and(|review| ours(&review)) {
            answer = Some(answer.unwrap_or(true));
        }
    }
    let undone = adjustments.iter().any(|adjustment| {
        adjustment.id().as_str().ends_with(UNDO_SUFFIX)
            && counted_review(adjustment.id()).is_some_and(|review| ours(&review))
    });
    if undone { Some(false) } else { answer }
}

/// The « Oui, je travaillais » in a row about `appointment_id`, up to `workday`, read from the
/// stored days of its weekday over [`TRUST_LOOKBACK_WEEKS`]; a day
/// without an absence in it neither counts nor breaks the streak.
pub fn streak(appointment_id: &str, workday: &LocalWorkday, connection: &Connection) -> usize {
    let history = DailySummaryRepository::new(connection);
    let adjustments = ManualAdjustmentRepository::new(connection);
    let mut streak = 0;
    for week in 0..TRUST_LOOKBACK_WEEKS {
        let Ok(date) = workday.date.checked_sub(week.weeks()) else {
            break;
        };
        let day = if week == 0 {
            Some(workday.clone())
        } else {
            history
                .read_latest(date)
                .ok()
                .flatten()
                .map(|revision| revision.snapshot().workday().clone())
        };
        let Some(day) = day.filter(LocalWorkday::is_working_day) else {
            continue;
        };
        let Ok(recorded) = adjustments.read_for_workday(&day) else {
            continue;
        };
        match answer(appointment_id, &recorded) {
            Some(true) => streak += 1,
            Some(false) => break,
            None => {}
        }
    }
    streak
}

/// How far back [`streak`] reads.
const TRUST_LOOKBACK_WEEKS: i64 = 8;

#[cfg(test)]
mod tests;
