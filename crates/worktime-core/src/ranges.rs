//! A range of a day counted as work or excluded from it at once, as the week's timeline selects
//! it. The range takes the user's corrections out of the long locks it touches and settles each
//! lock whole, then replaces what the user corrected in the rest by hand and corrects what the
//! day still counts the other way, all in one transaction: a refused range records nothing.

use jiff::Timestamp;
use worktime_domain::accounting::{
    AccountedSegment, AccountingLabel, AccountingResult, AdjustmentId, SegmentKind,
};
use worktime_domain::adjustments::{
    AdjustmentKind, InvalidAdjustment, ManualAdjustment, ManualClassification,
};
use worktime_domain::intervals::UtcInterval;
use worktime_domain::time::LocalWorkday;
use worktime_store::adjustments::{ManualAdjustmentError, ManualAdjustmentRepository};

use crate::accounting::{AccountingError, CalculationContext, WorkdayAccounting};
use crate::corrections::{Binding, binding, history_by_id};
use crate::reviews::{LongLockReviews, ManualResolution, ReviewError, resolution_interval};
use crate::weekly::start_of_week;

/// Counts `range` of `workday` as work, or excludes it, recorded under ids derived from `id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeCommand {
    pub workday: LocalWorkday,
    pub range: UtcInterval,
    pub classification: ManualClassification,
    pub id: AdjustmentId,
    pub created_at: Timestamp,
}

/// What a range recorded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RangeOutcome {
    /// The corrections the range added, which withdrawing undoes it while `replaced` is false.
    pub added: Vec<AdjustmentId>,
    /// The range settled a long lock or replaced a correction, which no withdrawal brings back.
    pub replaced: bool,
}

impl RangeOutcome {
    /// The corrections whose withdrawal undoes the range, when withdrawing them is enough.
    pub fn undoable(&self) -> Option<&[AdjustmentId]> {
        (!self.replaced && !self.added.is_empty()).then_some(self.added.as_slice())
    }
}

#[derive(Debug)]
pub enum RangeError {
    OutsideCurrentWeek,
    /// The range touches a settled long lock that counts the other way, whose correction stays
    /// bound to its review.
    Bound(AdjustmentId),
    Accounting(AccountingError),
    Adjustments(ManualAdjustmentError),
    Review(ReviewError),
    Invalid(InvalidAdjustment),
}

impl std::fmt::Display for RangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutsideCurrentWeek => {
                f.write_str("A range correction must target the current local week.")
            }
            Self::Bound(id) => write!(f, "the range overlaps the review-bound adjustment '{id}'"),
            Self::Accounting(error) => error.fmt(f),
            Self::Adjustments(error) => error.fmt(f),
            Self::Review(error) => error.fmt(f),
            Self::Invalid(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RangeError {}

impl From<AccountingError> for RangeError {
    fn from(error: AccountingError) -> Self {
        Self::Accounting(error)
    }
}

impl From<ManualAdjustmentError> for RangeError {
    fn from(error: ManualAdjustmentError) -> Self {
        Self::Adjustments(error)
    }
}

impl From<ReviewError> for RangeError {
    fn from(error: ReviewError) -> Self {
        Self::Review(error)
    }
}

impl From<InvalidAdjustment> for RangeError {
    fn from(error: InvalidAdjustment) -> Self {
        Self::Invalid(error)
    }
}

pub struct RangeCorrections<'a> {
    accounting: &'a WorkdayAccounting<'a>,
    adjustments: ManualAdjustmentRepository<'a>,
    reviews: LongLockReviews<'a>,
}

impl<'a> RangeCorrections<'a> {
    pub fn new(
        accounting: &'a WorkdayAccounting<'a>,
        adjustments: ManualAdjustmentRepository<'a>,
        reviews: LongLockReviews<'a>,
    ) -> Self {
        Self {
            accounting,
            adjustments,
            reviews,
        }
    }

    /// Records `command`, in one transaction. Each long lock the range touches is settled whole,
    /// the same way, before the rest: a lock is answered once. Asked again, the range finds what
    /// it recorded and adds nothing; refused, it records nothing. The ledger stays locked for
    /// writing while the day is calculated again, which the window's one connection never
    /// waits on.
    pub fn record(&self, command: &RangeCommand) -> Result<RangeOutcome, RangeError> {
        let context = self.accounting.capture_context();
        let today = context.calendar().workday(context.observed_at).date;
        let workday = &command.workday;
        if workday.date < start_of_week(today) || workday.date > today {
            return Err(RangeError::OutsideCurrentWeek);
        }
        let resolution = match command.classification {
            ManualClassification::Credited => ManualResolution::ConfirmWork,
            ManualClassification::Excluded => ManualResolution::Exclude,
        };
        // Every write below, the locks' among them, commits as one or not at all.
        let scope = self.adjustments.scope()?;

        let mut calculation = self.accounting.calculate(workday.date, &context)?;
        let current = calculation.summary_input.accounting();
        let (locks, resolved): (Vec<_>, Vec<_>) = current
            .pending_reviews()
            .iter()
            .filter_map(|review| {
                resolution_interval(current, review.locked_episode)
                    .filter(|resolved| resolved.overlaps(command.range))
                    .map(|resolved| (review, resolved))
            })
            .unzip();
        // A lock still going on can't be settled: refused before anything is written.
        if let Some(open) = locks.iter().find(|review| review.is_open) {
            return Err(RangeError::Review(ReviewError::StillOpen(open.id.clone())));
        }
        let locks: Vec<_> = locks.into_iter().map(|review| review.id.clone()).collect();
        let history = self.adjustments.read_for_workday(workday)?;
        // Refused on the day as it stands, before any lock is settled.
        plan(current, &history, command)?;
        // A lock is settled over the whole of it, so no correction of the user's may stay there.
        let cleared = clear(&history, &resolved, command)?;
        if !cleared.replacements.is_empty() {
            self.append_cut(workday, &context, current, &cleared)?;
        }
        for review in &locks {
            self.reviews
                .resolve(self.accounting, workday, review, resolution)?;
        }
        if !locks.is_empty() || !cleared.replacements.is_empty() {
            calculation = self.accounting.calculate(workday.date, &context)?;
        }

        // The corrections counting the other way are trimmed first: what they took from the
        // range, the day may then count as asked without a correction of its own.
        let current = calculation.summary_input.accounting();
        let history = self.adjustments.read_for_workday(workday)?;
        let trims = plan(current, &history, command)?;
        let trimmed = trims.replacements.len() + trims.rests.len();
        if trimmed > 0 {
            self.append_cut(workday, &context, current, &trims)?;
            calculation = self.accounting.calculate(workday.date, &context)?;
        }

        let current = calculation.summary_input.accounting();
        let history = self.adjustments.read_for_workday(workday)?;
        let fill = plan_from(current, &history, command, trimmed + 1)?;
        self.adjustments
            .append_all_validated(current, &fill.records())?;
        scope.commit().map_err(ManualAdjustmentError::from)?;
        Ok(RangeOutcome {
            added: trims
                .rests
                .iter()
                .chain(&fill.rests)
                .chain(&fill.additions)
                .map(|addition| addition.id().clone())
                .collect(),
            replaced: !locks.is_empty()
                || !cleared.replacements.is_empty()
                || !trims.replacements.is_empty()
                || !fill.replacements.is_empty(),
        })
    }

    /// Appends what replaces each correction `cut` trims on the day `current` accounts, then,
    /// on the day without them, the far ends it keeps: an interval of work kept there may not
    /// meet the time its original credits until that one is replaced.
    fn append_cut(
        &self,
        workday: &LocalWorkday,
        context: &CalculationContext,
        current: &AccountingResult,
        cut: &Plan,
    ) -> Result<(), RangeError> {
        self.adjustments
            .append_all_validated(current, &cut.replacements)?;
        if cut.rests.is_empty() {
            return Ok(());
        }
        let calculation = self.accounting.calculate(workday.date, context)?;
        let current = calculation.summary_input.accounting();
        let rests: Vec<_> = cut
            .rests
            .iter()
            .map(|rest| settled(rest, current))
            .collect::<Result<_, _>>()?;
        self.adjustments.append_all_validated(current, &rests)?;
        Ok(())
    }
}

/// The corrections a range records once its long locks are settled: what replaces each
/// correction it overrides, the far end of one it cuts in two, then what corrects the rest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub replacements: Vec<ManualAdjustment>,
    pub rests: Vec<ManualAdjustment>,
    pub additions: Vec<ManualAdjustment>,
}

impl Plan {
    pub fn records(&self) -> Vec<ManualAdjustment> {
        self.replacements
            .iter()
            .chain(&self.rests)
            .chain(&self.additions)
            .cloned()
            .collect()
    }
}

/// What `command` records on the day `current` accounts, whose corrections are `history`.
/// Within the accounted part of the range, a correction counting the other way is trimmed
/// to what lies outside it, or withdrawn; then every stretch the day still counts the other
/// way, or leaves to settle, gets a correction. A stretch already counted as asked is left as
/// it is, so a pause stays a pause.
pub fn plan(
    current: &AccountingResult,
    history: &[ManualAdjustment],
    command: &RangeCommand,
) -> Result<Plan, RangeError> {
    plan_from(current, history, command, 1)
}

/// [`plan`], numbering its corrections from `first`.
fn plan_from(
    current: &AccountingResult,
    history: &[ManualAdjustment],
    command: &RangeCommand,
    first: usize,
) -> Result<Plan, RangeError> {
    let timeline = current.timeline();
    let Some(range) = UtcInterval::new(timeline.window().start, timeline.through())
        .ok()
        .and_then(|horizon| horizon.intersect(command.range))
    else {
        return Ok(Plan::default());
    };
    let target = command.classification;
    let mut ids = (first..).map(|index| {
        AdjustmentId::new(format!("{}.{index}", command.id)).expect("the id is not blank")
    });
    let workday = &command.workday;
    let at = command.created_at;

    let by_id = history_by_id(history.iter().cloned());
    let superseded: Vec<&AdjustmentId> = history
        .iter()
        .filter_map(ManualAdjustment::supersedes_id)
        .collect();
    let mut replacements = Vec::new();
    let mut rests = Vec::new();
    let mut additions = Vec::new();
    for adjustment in history {
        if superseded.contains(&adjustment.id()) {
            continue;
        }
        let (Some(interval), Some(classification)) =
            (adjustment.interval(), classification_of(adjustment.kind()))
        else {
            continue;
        };
        if classification == target || !interval.overlaps(range) {
            continue;
        }
        if binding(adjustment, &by_id) != Binding::Free {
            return Err(RangeError::Bound(adjustment.id().clone()));
        }
        let before = UtcInterval::new(interval.start(), range.start()).ok();
        let after = UtcInterval::new(range.end(), interval.end()).ok();
        let id = ids.next().expect("the ids never end");
        let replacement = match before.or(after) {
            None => ManualAdjustment::revocation(id, workday.clone(), adjustment.id().clone(), at)?,
            Some(kept) => {
                with_interval(adjustment, kept, id, at)?.superseding(adjustment.id().clone())
            }
        };
        replacements.push(replacement.with_reason(adjustment.reason().unwrap_or_default()));
        if let (Some(_), Some(after)) = (before, after) {
            // The kept end becomes a correction of its own, recorded once its original is
            // replaced.
            let rest = rest_of(
                adjustment,
                after,
                ids.next().expect("the ids never end"),
                at,
            )?;
            rests.push(rest.with_reason(adjustment.reason().unwrap_or_default()));
        }
    }

    // What a correction already counts as asked, as a confirmed lock does the sleep within it,
    // stays as the ledger leaves it: a second correction there would overlap the first.
    let covered: Vec<UtcInterval> = history
        .iter()
        .filter(|adjustment| !superseded.contains(&adjustment.id()))
        .filter(|adjustment| classification_of(adjustment.kind()) == Some(target))
        .filter_map(ManualAdjustment::interval)
        .collect();
    let stretches = stretches(timeline.segments(), range, target)
        .into_iter()
        .flat_map(|stretch| outside(stretch, &covered));
    for stretch in stretches {
        let id = ids.next().expect("the ids never end");
        additions.push(match target {
            ManualClassification::Credited => {
                ManualAdjustment::interval_work(id, workday.clone(), stretch, at)?
            }
            ManualClassification::Excluded => {
                ManualAdjustment::exclusion(id, workday.clone(), stretch, at)?
            }
        });
    }
    Ok(Plan {
        replacements,
        rests,
        additions,
    })
}

/// What takes the user's corrections out of `locks`, whichever way they count: each one that
/// meets a lock is replaced by what lies outside them, and outside the range when it counts
/// the other way, or withdrawn.
fn clear(
    history: &[ManualAdjustment],
    locks: &[UtcInterval],
    command: &RangeCommand,
) -> Result<Plan, RangeError> {
    let mut ids = (1..).map(|index| {
        AdjustmentId::new(format!("{}.lock.{index}", command.id)).expect("the id is not blank")
    });
    let (workday, at) = (&command.workday, command.created_at);
    let by_id = history_by_id(history.iter().cloned());
    let superseded: Vec<&AdjustmentId> = history
        .iter()
        .filter_map(ManualAdjustment::supersedes_id)
        .collect();
    let mut replacements = Vec::new();
    let mut rests = Vec::new();
    for adjustment in history {
        let (Some(interval), Some(classification)) =
            (adjustment.interval(), classification_of(adjustment.kind()))
        else {
            continue;
        };
        if superseded.contains(&adjustment.id())
            || !locks.iter().any(|lock| lock.overlaps(interval))
        {
            continue;
        }
        if binding(adjustment, &by_id) != Binding::Free {
            return Err(RangeError::Bound(adjustment.id().clone()));
        }
        let reason = adjustment.reason().unwrap_or_default();
        // One counting the other way loses the range too, now: its successor here is the last
        // the range records, as no second one at the same instant could follow it.
        let mut cuts = locks.to_vec();
        if classification != command.classification {
            cuts.push(command.range);
        }
        let mut kept = outside(interval, &cuts).into_iter();
        let id = ids.next().expect("the ids never end");
        replacements.push(
            match kept.next() {
                None => {
                    ManualAdjustment::revocation(id, workday.clone(), adjustment.id().clone(), at)?
                }
                Some(first) => {
                    with_interval(adjustment, first, id, at)?.superseding(adjustment.id().clone())
                }
            }
            .with_reason(reason),
        );
        for rest in kept {
            let id = ids.next().expect("the ids never end");
            rests.push(rest_of(adjustment, rest, id, at)?.with_reason(reason));
        }
    }
    Ok(Plan {
        replacements,
        rests,
        additions: Vec::new(),
    })
}

/// The far end of `adjustment` over `interval`, a correction of its own under `id`, of the same
/// kind. A lock's confirmation stays with its replacement: the end kept only counts the same
/// way.
fn rest_of(
    adjustment: &ManualAdjustment,
    interval: UtcInterval,
    id: AdjustmentId,
    at: Timestamp,
) -> Result<ManualAdjustment, InvalidAdjustment> {
    match adjustment.kind() {
        AdjustmentKind::Reclassification { classification, .. } => {
            ManualAdjustment::reclassification(
                id,
                adjustment.workday().clone(),
                interval,
                *classification,
                at,
            )
        }
        _ => with_interval(adjustment, interval, id, at),
    }
}

/// `rest` as `current` lets the day record it: an interval of work over time the day credits
/// already, as activity there, counts it as a reclassification instead.
fn settled(
    rest: &ManualAdjustment,
    current: &AccountingResult,
) -> Result<ManualAdjustment, InvalidAdjustment> {
    let AdjustmentKind::IntervalWork { interval } = rest.kind() else {
        return Ok(rest.clone());
    };
    let credited = current
        .timeline()
        .segments()
        .iter()
        .any(|segment| segment.kind().is_credit() && segment.interval().overlaps(*interval));
    if !credited {
        return Ok(rest.clone());
    }
    Ok(ManualAdjustment::reclassification(
        rest.id().clone(),
        rest.workday().clone(),
        *interval,
        ManualClassification::Credited,
        rest.created_at(),
    )?
    .with_reason(rest.reason().unwrap_or_default()))
}

/// What of `interval` lies outside every one of `cuts`, in order.
fn outside(interval: UtcInterval, cuts: &[UtcInterval]) -> Vec<UtcInterval> {
    let mut pieces = vec![interval];
    for cut in cuts {
        pieces = pieces
            .into_iter()
            .flat_map(|piece| {
                if piece.overlaps(*cut) {
                    vec![
                        UtcInterval::new(piece.start(), cut.start()).ok(),
                        UtcInterval::new(cut.end(), piece.end()).ok(),
                    ]
                } else {
                    vec![Some(piece)]
                }
            })
            .flatten()
            .collect();
    }
    pieces
}

/// How an interval correction counts its time; `None` for one without an interval.
fn classification_of(kind: &AdjustmentKind) -> Option<ManualClassification> {
    match kind {
        AdjustmentKind::IntervalWork { .. } => Some(ManualClassification::Credited),
        AdjustmentKind::Exclusion { .. } => Some(ManualClassification::Excluded),
        AdjustmentKind::Reclassification { classification, .. } => Some(*classification),
        AdjustmentKind::DurationWork { .. } | AdjustmentKind::Revocation => None,
    }
}

/// `adjustment` over `interval` instead, under `id`.
fn with_interval(
    adjustment: &ManualAdjustment,
    interval: UtcInterval,
    id: AdjustmentId,
    at: Timestamp,
) -> Result<ManualAdjustment, InvalidAdjustment> {
    let kind = match adjustment.kind() {
        AdjustmentKind::IntervalWork { .. } => AdjustmentKind::IntervalWork { interval },
        AdjustmentKind::Exclusion { .. } => AdjustmentKind::Exclusion { interval },
        AdjustmentKind::Reclassification {
            classification,
            confirms_review_id,
            ..
        } => AdjustmentKind::Reclassification {
            interval,
            classification: *classification,
            confirms_review_id: confirms_review_id.clone(),
        },
        AdjustmentKind::DurationWork { .. } | AdjustmentKind::Revocation => {
            unreachable!("only interval corrections overlap a range")
        }
    };
    ManualAdjustment::new(id, adjustment.workday().clone(), at, None, kind)
}

/// The stretches of `range` that `segments` do not count as `target` asks, joined when they
/// touch. Excluding settles time left unknown or asleep too.
fn stretches(
    segments: &[AccountedSegment],
    range: UtcInterval,
    target: ManualClassification,
) -> Vec<UtcInterval> {
    let mut stretches: Vec<UtcInterval> = Vec::new();
    for segment in segments {
        let Some(piece) = segment.interval().intersect(range) else {
            continue;
        };
        if !differs(segment, target) {
            continue;
        }
        match stretches.last_mut() {
            Some(last) if last.end() == piece.start() => {
                *last = UtcInterval::new(last.start(), piece.end()).expect("the pieces are sorted");
            }
            _ => stretches.push(piece),
        }
    }
    stretches
}

fn differs(segment: &AccountedSegment, target: ManualClassification) -> bool {
    let credited = segment.kind().is_credit();
    match target {
        ManualClassification::Credited => !credited,
        ManualClassification::Excluded => {
            credited
                || *segment.kind() == SegmentKind::Unknown
                || (segment.origin().manual_adjustment_id().is_none()
                    && matches!(
                        segment.label(),
                        AccountingLabel::Suspended | AccountingLabel::OvernightLock
                    ))
        }
    }
}

#[cfg(test)]
mod tests;
