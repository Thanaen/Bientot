//! The accounted timeline of a workday and the result of accounting it.

use std::collections::BTreeMap;

use jiff::{SignedDuration, Timestamp};

use super::evidence::{AutomaticOrigin, WorkdayEventId};
use crate::configuration::RecurringAppointment;
use crate::intervals::{IntervalSet, UtcInterval};
use crate::time::WorkdayWindow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AccountingLabel {
    ActiveWork,
    Afk,
    LunchLock,
    LockedWork,
    Suspended,
    ManualPause,
    DayFinished,
    Manual,
    Unknown,
    OvernightLock,
    /// Away from the PC during a recurring appointment.
    ScheduledAway,
    /// The session was open before Bientôt watched the activity, as the Windows logs tell.
    RebuiltWork,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlankIdentifier;

impl std::fmt::Display for BlankIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an identifier cannot be blank")
    }
}

impl std::error::Error for BlankIdentifier {}

pub(crate) fn non_blank(value: String) -> Result<String, BlankIdentifier> {
    if value.trim().is_empty() {
        Err(BlankIdentifier)
    } else {
        Ok(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AdjustmentId(String);

impl AdjustmentId {
    pub fn new(value: impl Into<String>) -> Result<Self, BlankIdentifier> {
        non_blank(value.into()).map(|value| Self(value.trim().to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AdjustmentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Unlike [`AdjustmentId`], the value is kept untrimmed, as in the .NET app.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LongLockReviewId(String);

impl LongLockReviewId {
    pub fn new(value: impl Into<String>) -> Result<Self, BlankIdentifier> {
        non_blank(value.into()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const APPOINTMENT_REVIEW_PREFIX: &str = "recurring-appointment:";

impl LongLockReviewId {
    /// The review of the absence from `start` during the appointment `appointment_id`, whose
    /// identifier holds no colon.
    pub fn for_appointment(appointment_id: &str, start: Timestamp) -> Self {
        Self::new(format!(
            "{APPOINTMENT_REVIEW_PREFIX}{appointment_id}:{start}"
        ))
        .expect("the id is not blank")
    }

    /// The appointment of a review [`Self::for_appointment`] named.
    pub fn appointment_id(&self) -> Option<&str> {
        let rest = self.0.strip_prefix(APPOINTMENT_REVIEW_PREFIX)?;
        rest.split_once(':').map(|(id, _)| id)
    }
}

const REBUILT_SESSION_REVIEW_PREFIX: &str = "session-rebuilt:";

impl LongLockReviewId {
    /// The review of the time the session was open from `start`, after the unlock recorded as
    /// `unlock`.
    pub fn for_rebuilt_session(unlock: WorkdayEventId, start: Timestamp) -> Self {
        Self::new(format!(
            "{REBUILT_SESSION_REVIEW_PREFIX}{}:{start}",
            unlock.0
        ))
        .expect("the id is not blank")
    }
}

impl std::fmt::Display for LongLockReviewId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AccountedOrigin {
    Automatic(AutomaticOrigin),
    /// A manual adjustment, and the origin of the accounting it overrode.
    Manual {
        adjustment_id: AdjustmentId,
        replaces: Box<AccountedOrigin>,
    },
}

impl AccountedOrigin {
    pub fn manual_adjustment_id(&self) -> Option<&AdjustmentId> {
        match self {
            Self::Automatic(_) => None,
            Self::Manual { adjustment_id, .. } => Some(adjustment_id),
        }
    }
}

impl From<AutomaticOrigin> for AccountedOrigin {
    fn from(origin: AutomaticOrigin) -> Self {
        Self::Automatic(origin)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SegmentKind {
    Credited,
    /// Credited until the long-lock review it belongs to is resolved.
    ProvisionalCredit {
        review_id: LongLockReviewId,
    },
    Excluded,
    Unknown,
}

impl SegmentKind {
    pub fn is_credit(&self) -> bool {
        matches!(self, Self::Credited | Self::ProvisionalCredit { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AccountedSegment {
    interval: UtcInterval,
    kind: SegmentKind,
    label: AccountingLabel,
    origin: AccountedOrigin,
}

impl AccountedSegment {
    pub fn credited(
        interval: UtcInterval,
        label: AccountingLabel,
        origin: impl Into<AccountedOrigin>,
    ) -> Self {
        Self::of_kind(SegmentKind::Credited, interval, label, origin.into())
    }

    pub fn provisional_credit(
        interval: UtcInterval,
        label: AccountingLabel,
        origin: impl Into<AccountedOrigin>,
        review_id: LongLockReviewId,
    ) -> Self {
        Self::of_kind(
            SegmentKind::ProvisionalCredit { review_id },
            interval,
            label,
            origin.into(),
        )
    }

    pub fn excluded(
        interval: UtcInterval,
        label: AccountingLabel,
        origin: impl Into<AccountedOrigin>,
    ) -> Self {
        Self::of_kind(SegmentKind::Excluded, interval, label, origin.into())
    }

    pub fn unknown(interval: UtcInterval, origin: impl Into<AccountedOrigin>) -> Self {
        Self::of_kind(
            SegmentKind::Unknown,
            interval,
            AccountingLabel::Unknown,
            origin.into(),
        )
    }

    fn of_kind(
        kind: SegmentKind,
        interval: UtcInterval,
        label: AccountingLabel,
        origin: AccountedOrigin,
    ) -> Self {
        Self {
            interval,
            kind,
            label,
            origin,
        }
    }

    pub fn interval(&self) -> UtcInterval {
        self.interval
    }

    pub fn kind(&self) -> &SegmentKind {
        &self.kind
    }

    pub fn label(&self) -> AccountingLabel {
        self.label
    }

    pub fn origin(&self) -> &AccountedOrigin {
        &self.origin
    }

    pub(crate) fn with_interval(&self, interval: UtcInterval) -> Self {
        Self {
            interval,
            ..self.clone()
        }
    }

    fn continues_into(&self, next: &Self) -> bool {
        self.interval.end() == next.interval.start()
            && self.kind == next.kind
            && self.label == next.label
            && self.origin == next.origin
    }
}

/// Appends `next`, extending the last segment instead when `next` continues it unchanged.
pub(crate) fn append_canonical(segments: &mut Vec<AccountedSegment>, next: AccountedSegment) {
    match segments.last_mut() {
        Some(previous) if previous.continues_into(&next) => {
            previous.interval = UtcInterval::new(previous.interval.start(), next.interval.end())
                .expect("adjacent segments span a positive duration");
        }
        _ => segments.push(next),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LongLockReviewReason {
    ThresholdExceeded,
    /// Away from the PC during a recurring appointment, however long.
    RecurringAppointment,
    /// The session was open before Bientôt watched the activity, however long.
    RebuiltSession,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LongLockReview {
    pub id: LongLockReviewId,
    pub locked_episode: UtcInterval,
    pub provisional_coverage: IntervalSet,
    pub reason: LongLockReviewReason,
    pub is_open: bool,
}

impl LongLockReview {
    /// The name the user gave the recurring appointment the absence fell in, empty for none,
    /// among `appointments`, from which it may be gone since; `None` for a lock.
    pub fn appointment_name(&self, appointments: &[RecurringAppointment]) -> Option<String> {
        (self.reason == LongLockReviewReason::RecurringAppointment).then(|| {
            let id = self.id.appointment_id();
            appointments
                .iter()
                .find(|appointment| Some(appointment.id.as_str()) == id)
                .map(|appointment| appointment.name.clone())
                .unwrap_or_default()
        })
    }
}

/// Sorted, adjacent, disjoint segments covering `[window.start, through)` exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountingTimeline {
    window: WorkdayWindow,
    through: Timestamp,
    segments: Vec<AccountedSegment>,
}

impl AccountingTimeline {
    /// # Panics
    ///
    /// When the window is empty, `through` lies outside it, or the segments do not cover
    /// `[window.start, through)` exactly and in order.
    pub(crate) fn new(
        window: WorkdayWindow,
        through: Timestamp,
        segments: Vec<AccountedSegment>,
    ) -> Self {
        assert!(
            window.start < window.end,
            "the workday window must have a positive duration"
        );
        assert!(
            window.start <= through && through <= window.end,
            "the accounting horizon must lie within the workday window"
        );
        let mut expected_start = window.start;
        for segment in &segments {
            assert_eq!(
                segment.interval.start(),
                expected_start,
                "segments must be sorted, adjacent and pairwise disjoint from the window start"
            );
            expected_start = segment.interval.end();
        }
        assert_eq!(
            expected_start, through,
            "segments must cover the accounting horizon exactly"
        );
        Self {
            window,
            through,
            segments,
        }
    }

    pub fn window(&self) -> &WorkdayWindow {
        &self.window
    }

    pub fn through(&self) -> Timestamp {
        self.through
    }

    pub fn segments(&self) -> &[AccountedSegment] {
        &self.segments
    }

    pub fn credited_work(&self) -> SignedDuration {
        self.sum_duration(SegmentKind::is_credit)
    }

    pub fn confirmed_credited_work(&self) -> SignedDuration {
        self.sum_duration(|kind| matches!(kind, SegmentKind::Credited))
    }

    pub fn provisional_work(&self) -> SignedDuration {
        self.sum_duration(|kind| matches!(kind, SegmentKind::ProvisionalCredit { .. }))
    }

    pub fn excluded_work(&self) -> SignedDuration {
        self.sum_duration(|kind| matches!(kind, SegmentKind::Excluded))
    }

    pub fn unknown_work(&self) -> SignedDuration {
        self.sum_duration(|kind| matches!(kind, SegmentKind::Unknown))
    }

    pub fn credited_coverage(&self) -> IntervalSet {
        self.segments
            .iter()
            .filter(|segment| segment.kind.is_credit())
            .map(|segment| segment.interval)
            .collect()
    }

    pub(crate) fn provisional_coverage_by_review(
        &self,
    ) -> BTreeMap<&LongLockReviewId, IntervalSet> {
        let mut coverage: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for segment in &self.segments {
            if let SegmentKind::ProvisionalCredit { review_id } = &segment.kind {
                coverage
                    .entry(review_id)
                    .or_default()
                    .push(segment.interval);
            }
        }
        coverage
            .into_iter()
            .map(|(review_id, intervals)| (review_id, IntervalSet::new(intervals)))
            .collect()
    }

    fn sum_duration(&self, predicate: impl Fn(&SegmentKind) -> bool) -> SignedDuration {
        self.segments
            .iter()
            .filter(|segment| predicate(&segment.kind))
            .fold(SignedDuration::ZERO, |total, segment| {
                total + segment.interval.duration()
            })
    }
}

/// Work declared as a duration, counted outside the timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManualDurationCredit {
    adjustment_id: AdjustmentId,
    duration: SignedDuration,
}

impl ManualDurationCredit {
    pub(crate) fn new(adjustment_id: AdjustmentId, duration: SignedDuration) -> Self {
        Self {
            adjustment_id,
            duration,
        }
    }

    pub fn adjustment_id(&self) -> &AdjustmentId {
        &self.adjustment_id
    }

    pub fn duration(&self) -> SignedDuration {
        self.duration
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountingResult {
    timeline: AccountingTimeline,
    manual_duration_credits: Vec<ManualDurationCredit>,
    pending_reviews: Vec<LongLockReview>,
}

impl AccountingResult {
    pub(crate) fn new(
        timeline: AccountingTimeline,
        manual_duration_credits: Vec<ManualDurationCredit>,
        pending_reviews: Vec<LongLockReview>,
    ) -> Self {
        Self {
            timeline,
            manual_duration_credits,
            pending_reviews,
        }
    }

    /// The same result with other pending reviews, as the .NET record constructor allows: the
    /// app's snapshot tests use it to check that the order of the reviews is not material.
    pub fn with_pending_reviews(self, pending_reviews: Vec<LongLockReview>) -> Self {
        Self {
            pending_reviews,
            ..self
        }
    }

    pub fn timeline(&self) -> &AccountingTimeline {
        &self.timeline
    }

    pub fn manual_duration_credits(&self) -> &[ManualDurationCredit] {
        &self.manual_duration_credits
    }

    pub fn pending_reviews(&self) -> &[LongLockReview] {
        &self.pending_reviews
    }

    pub fn work_awaiting_review(&self) -> SignedDuration {
        self.pending_reviews
            .iter()
            .fold(SignedDuration::ZERO, |total, review| {
                total + review.provisional_coverage.covered_duration()
            })
    }

    pub fn credited_work(&self) -> SignedDuration {
        self.manual_duration_credits
            .iter()
            .fold(self.timeline.credited_work(), |total, credit| {
                total + credit.duration
            })
    }
}
