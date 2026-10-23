use jiff::{SignedDuration, Timestamp};

use crate::accounting::{AccountingLabel, AdjustmentId, LongLockReviewId};
use crate::intervals::UtcInterval;
use crate::sources::SourceQuality;
use crate::summaries::IndicatorState;
use crate::time::LocalWorkday;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DailySummaryDisposition {
    ConfirmedCredit,
    ProvisionalCredit,
    Excluded,
    Unknown,
    ManualCredit,
}

impl DailySummaryDisposition {
    fn is_credit(self) -> bool {
        matches!(
            self,
            Self::ConfirmedCredit | Self::ProvisionalCredit | Self::ManualCredit
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidBreakdownEntry {
    NonPositiveDuration,
    IntervalDurationMismatch,
}

impl std::fmt::Display for InvalidBreakdownEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NonPositiveDuration => "breakdown duration must be positive",
            Self::IntervalDurationMismatch => "breakdown interval and duration must agree",
        })
    }
}

impl std::error::Error for InvalidBreakdownEntry {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DailySummaryBreakdownEntry {
    disposition: DailySummaryDisposition,
    label: AccountingLabel,
    duration: SignedDuration,
    interval: Option<UtcInterval>,
    adjustment_id: Option<AdjustmentId>,
    review_id: Option<LongLockReviewId>,
}

impl DailySummaryBreakdownEntry {
    pub fn new(
        disposition: DailySummaryDisposition,
        label: AccountingLabel,
        duration: SignedDuration,
        interval: Option<UtcInterval>,
        adjustment_id: Option<AdjustmentId>,
        review_id: Option<LongLockReviewId>,
    ) -> Result<Self, InvalidBreakdownEntry> {
        if !duration.is_positive() {
            return Err(InvalidBreakdownEntry::NonPositiveDuration);
        }
        if interval.is_some_and(|interval| interval.duration() != duration) {
            return Err(InvalidBreakdownEntry::IntervalDurationMismatch);
        }
        Ok(Self {
            disposition,
            label,
            duration,
            interval,
            adjustment_id,
            review_id,
        })
    }

    pub fn disposition(&self) -> DailySummaryDisposition {
        self.disposition
    }

    pub fn label(&self) -> AccountingLabel {
        self.label
    }

    pub fn duration(&self) -> SignedDuration {
        self.duration
    }

    pub fn interval(&self) -> Option<UtcInterval> {
        self.interval
    }

    pub fn adjustment_id(&self) -> Option<&AdjustmentId> {
        self.adjustment_id.as_ref()
    }

    pub fn review_id(&self) -> Option<&LongLockReviewId> {
        self.review_id.as_ref()
    }
}

/// Breakdown entries in a stable order: timed entries by interval first, then untimed ones.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DailySummaryBreakdown {
    entries: Vec<DailySummaryBreakdownEntry>,
}

impl DailySummaryBreakdown {
    pub fn new(entries: impl IntoIterator<Item = DailySummaryBreakdownEntry>) -> Self {
        let mut entries: Vec<_> = entries.into_iter().collect();
        entries.sort_by(|left, right| sort_key(left).cmp(&sort_key(right)));
        Self { entries }
    }

    pub fn entries(&self) -> &[DailySummaryBreakdownEntry] {
        &self.entries
    }

    fn total(&self, included: impl Fn(DailySummaryDisposition) -> bool) -> SignedDuration {
        self.entries
            .iter()
            .filter(|entry| included(entry.disposition))
            .map(DailySummaryBreakdownEntry::duration)
            .sum()
    }
}

fn sort_key(
    entry: &DailySummaryBreakdownEntry,
) -> (
    bool,
    Option<UtcInterval>,
    DailySummaryDisposition,
    AccountingLabel,
    Option<&str>,
    Option<&str>,
    SignedDuration,
) {
    (
        entry.interval.is_none(),
        entry.interval,
        entry.disposition,
        entry.label,
        entry.adjustment_id.as_ref().map(AdjustmentId::as_str),
        entry.review_id.as_ref().map(LongLockReviewId::as_str),
        entry.duration,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidSnapshot {
    BlankTimeZone,
    NotAWorkingDay,
    BlankFingerprint,
    NegativeDuration,
    NonPositiveNominalTarget,
    EffectiveTargetOutOfRange,
    AppliedCarryMismatch,
    IndicatorContradictsCompletion,
    ProjectionContradictsCompletion,
    BreakdownMismatch,
}

impl std::fmt::Display for InvalidSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BlankTimeZone => "the workday time zone cannot be blank",
            Self::NotAWorkingDay => {
                "daily summary history is limited to Monday-to-Friday workdays"
            }
            Self::BlankFingerprint => "the input fingerprint cannot be blank",
            Self::NegativeDuration => "summary durations cannot be negative",
            Self::NonPositiveNominalTarget => "the nominal target must be positive",
            Self::EffectiveTargetOutOfRange => {
                "the effective target must be positive and at most the nominal target"
            }
            Self::AppliedCarryMismatch => {
                "applied carry must equal the reduction from nominal to effective target"
            }
            Self::IndicatorContradictsCompletion => {
                "blue must be used exactly when credited work reaches the effective target"
            }
            Self::ProjectionContradictsCompletion => {
                "incomplete summaries require a projected departure and complete summaries forbid one"
            }
            Self::BreakdownMismatch => {
                "breakdown disposition totals must match the stored credited, excluded, and unknown totals"
            }
        })
    }
}

impl std::error::Error for InvalidSnapshot {}

/// The stored values of a [`DailySummarySnapshot`], checked by [`DailySummarySnapshot::new`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DailySummarySnapshotParts {
    pub workday: LocalWorkday,
    pub computed_at: Timestamp,
    pub input_fingerprint: String,
    pub credited_work: SignedDuration,
    pub excluded_work: SignedDuration,
    pub unknown_work: SignedDuration,
    pub nominal_target: SignedDuration,
    pub effective_target: SignedDuration,
    pub carry_available: SignedDuration,
    pub carry_applied: SignedDuration,
    pub carry_earned: SignedDuration,
    pub carry_expired: SignedDuration,
    pub carry_debt: SignedDuration,
    /// Debt is on and the day was still running, so its shortfall is not in the carry debt yet.
    pub debt_pending: bool,
    pub projected_departure: Option<Timestamp>,
    pub indicator_state: IndicatorState,
    pub source_quality: SourceQuality,
    pub has_pending_review: bool,
    pub breakdown: DailySummaryBreakdown,
}

/// A daily summary as stored in history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DailySummarySnapshot {
    parts: DailySummarySnapshotParts,
}

impl DailySummarySnapshot {
    pub fn new(parts: DailySummarySnapshotParts) -> Result<Self, InvalidSnapshot> {
        if parts.workday.time_zone_id.trim().is_empty() {
            return Err(InvalidSnapshot::BlankTimeZone);
        }
        if !parts.workday.is_working_day() {
            return Err(InvalidSnapshot::NotAWorkingDay);
        }
        if parts.input_fingerprint.trim().is_empty() {
            return Err(InvalidSnapshot::BlankFingerprint);
        }
        if [parts.credited_work, parts.excluded_work, parts.unknown_work]
            .iter()
            .any(SignedDuration::is_negative)
        {
            return Err(InvalidSnapshot::NegativeDuration);
        }
        if !parts.nominal_target.is_positive() {
            return Err(InvalidSnapshot::NonPositiveNominalTarget);
        }
        if !parts.effective_target.is_positive() || parts.effective_target > parts.nominal_target {
            return Err(InvalidSnapshot::EffectiveTargetOutOfRange);
        }
        if [
            parts.carry_available,
            parts.carry_applied,
            parts.carry_earned,
            parts.carry_expired,
            parts.carry_debt,
        ]
        .iter()
        .any(SignedDuration::is_negative)
        {
            return Err(InvalidSnapshot::NegativeDuration);
        }
        if parts.carry_applied != parts.nominal_target - parts.effective_target {
            return Err(InvalidSnapshot::AppliedCarryMismatch);
        }
        let is_complete = parts.credited_work >= parts.effective_target;
        if is_complete != (parts.indicator_state == IndicatorState::Blue) {
            return Err(InvalidSnapshot::IndicatorContradictsCompletion);
        }
        if is_complete == parts.projected_departure.is_some() {
            return Err(InvalidSnapshot::ProjectionContradictsCompletion);
        }
        let breakdown = &parts.breakdown;
        if breakdown.total(DailySummaryDisposition::is_credit) != parts.credited_work
            || breakdown.total(|kind| kind == DailySummaryDisposition::Excluded)
                != parts.excluded_work
            || breakdown.total(|kind| kind == DailySummaryDisposition::Unknown)
                != parts.unknown_work
        {
            return Err(InvalidSnapshot::BreakdownMismatch);
        }
        Ok(Self { parts })
    }

    pub fn workday(&self) -> &LocalWorkday {
        &self.parts.workday
    }

    pub fn computed_at(&self) -> Timestamp {
        self.parts.computed_at
    }

    pub fn input_fingerprint(&self) -> &str {
        &self.parts.input_fingerprint
    }

    pub fn credited_work(&self) -> SignedDuration {
        self.parts.credited_work
    }

    pub fn excluded_work(&self) -> SignedDuration {
        self.parts.excluded_work
    }

    pub fn unknown_work(&self) -> SignedDuration {
        self.parts.unknown_work
    }

    pub fn nominal_target(&self) -> SignedDuration {
        self.parts.nominal_target
    }

    pub fn effective_target(&self) -> SignedDuration {
        self.parts.effective_target
    }

    pub fn carry_available(&self) -> SignedDuration {
        self.parts.carry_available
    }

    pub fn carry_applied(&self) -> SignedDuration {
        self.parts.carry_applied
    }

    pub fn carry_earned(&self) -> SignedDuration {
        self.parts.carry_earned
    }

    pub fn carry_expired(&self) -> SignedDuration {
        self.parts.carry_expired
    }

    pub fn carry_debt(&self) -> SignedDuration {
        self.parts.carry_debt
    }

    /// Debt is on and the day was still running, so its shortfall is not in the carry debt yet.
    pub fn debt_pending(&self) -> bool {
        self.parts.debt_pending
    }

    pub fn projected_departure(&self) -> Option<Timestamp> {
        self.parts.projected_departure
    }

    pub fn indicator_state(&self) -> IndicatorState {
        self.parts.indicator_state
    }

    pub fn source_quality(&self) -> SourceQuality {
        self.parts.source_quality
    }

    pub fn has_pending_review(&self) -> bool {
        self.parts.has_pending_review
    }

    pub fn breakdown(&self) -> &DailySummaryBreakdown {
        &self.parts.breakdown
    }

    /// Available credit minus debt at the end of the day; negative when the week owes time.
    pub fn net_carry_balance(&self) -> SignedDuration {
        self.parts.carry_available - self.parts.carry_debt
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NonPositiveRevision;

impl std::fmt::Display for NonPositiveRevision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a summary revision must be positive")
    }
}

impl std::error::Error for NonPositiveRevision {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DailySummaryRevision {
    revision: i64,
    snapshot: DailySummarySnapshot,
}

impl DailySummaryRevision {
    pub fn new(revision: i64, snapshot: DailySummarySnapshot) -> Result<Self, NonPositiveRevision> {
        if revision <= 0 {
            return Err(NonPositiveRevision);
        }
        Ok(Self { revision, snapshot })
    }

    pub fn revision(&self) -> i64 {
        self.revision
    }

    pub fn snapshot(&self) -> &DailySummarySnapshot {
        &self.snapshot
    }

    pub fn workday_date(&self) -> jiff::civil::Date {
        self.snapshot.parts.workday.date
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        disposition: DailySummaryDisposition,
        minutes: i64,
        start_minute: Option<i64>,
    ) -> DailySummaryBreakdownEntry {
        let duration = SignedDuration::from_mins(minutes);
        let interval = start_minute.map(|start| {
            let start = Timestamp::UNIX_EPOCH + SignedDuration::from_mins(start);
            UtcInterval::new(start, start + duration).unwrap()
        });
        DailySummaryBreakdownEntry::new(
            disposition,
            AccountingLabel::ActiveWork,
            duration,
            interval,
            None,
            None,
        )
        .unwrap()
    }

    #[test]
    fn breakdown_orders_timed_entries_by_interval_before_untimed_ones() {
        let manual = entry(DailySummaryDisposition::ManualCredit, 30, None);
        let late = entry(DailySummaryDisposition::Excluded, 10, Some(60));
        let early = entry(DailySummaryDisposition::ConfirmedCredit, 20, Some(0));

        let breakdown = DailySummaryBreakdown::new([manual.clone(), late.clone(), early.clone()]);

        assert_eq!(breakdown.entries(), [early, late, manual]);
    }

    #[test]
    fn breakdown_entries_need_a_positive_duration_that_agrees_with_their_interval() {
        let start = Timestamp::UNIX_EPOCH;
        let hour = UtcInterval::new(start, start + SignedDuration::from_hours(1)).unwrap();
        let entry = |duration, interval| {
            DailySummaryBreakdownEntry::new(
                DailySummaryDisposition::Unknown,
                AccountingLabel::Unknown,
                duration,
                interval,
                None,
                None,
            )
        };

        assert_eq!(
            entry(SignedDuration::ZERO, None),
            Err(InvalidBreakdownEntry::NonPositiveDuration)
        );
        assert_eq!(
            entry(SignedDuration::from_mins(59), Some(hour)),
            Err(InvalidBreakdownEntry::IntervalDurationMismatch)
        );
    }
}
