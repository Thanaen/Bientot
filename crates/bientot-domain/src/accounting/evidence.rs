//! Activity evidence reported by the sources.

use jiff::{SignedDuration, Timestamp};

use super::model::{BlankIdentifier, non_blank};
use crate::adjustments::ManualAdjustment;
use crate::intervals::UtcInterval;
use crate::time::WorkdayWindow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActivityState {
    Active,
    Afk,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlankSourceId;

impl std::fmt::Display for BlankSourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a source identifier cannot be blank")
    }
}

impl std::error::Error for BlankSourceId {}

/// The origin of the unknown time a source leaves uncovered, filled in before accounting. It
/// keeps the .NET identifier whatever the source, because accounted segments carry it into the
/// input fingerprints the ledger stores.
pub const COVERAGE_GAP_ORIGIN: &str = "activitywatch:coverage-gap";

/// The origin of a session transition read from the Windows logs after the fact, rather than
/// observed as it happened.
pub const SESSION_HISTORY_SOURCE_ID: &str = "windows-session-history";

/// The source that produced a piece of automatic evidence.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AutomaticOrigin(String);

impl AutomaticOrigin {
    pub fn new(source_id: impl Into<String>) -> Result<Self, BlankSourceId> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() {
            return Err(BlankSourceId);
        }
        Ok(Self(source_id))
    }

    pub fn source_id(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivitySegment {
    pub interval: UtcInterval,
    pub state: ActivityState,
    pub origin: AutomaticOrigin,
}

impl ActivitySegment {
    pub fn new(interval: UtcInterval, state: ActivityState, origin: AutomaticOrigin) -> Self {
        Self {
            interval,
            state,
            origin,
        }
    }
}

/// Activity evidence clipped to `bounds`, sorted by start, end and source. Segments may overlap
/// and may leave gaps; [`crate::sources::complete_unknown_gaps`] fills the gaps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityCoverage {
    bounds: UtcInterval,
    segments: Vec<ActivitySegment>,
}

impl ActivityCoverage {
    pub fn new(bounds: UtcInterval, segments: impl IntoIterator<Item = ActivitySegment>) -> Self {
        let mut segments: Vec<_> = segments
            .into_iter()
            .filter_map(|segment| {
                let interval = segment.interval.intersect(bounds)?;
                Some(ActivitySegment {
                    interval,
                    ..segment
                })
            })
            .collect();
        segments.sort_by(|left, right| {
            (
                left.interval.start(),
                left.interval.end(),
                left.origin.source_id(),
            )
                .cmp(&(
                    right.interval.start(),
                    right.interval.end(),
                    right.origin.source_id(),
                ))
        });
        Self { bounds, segments }
    }

    pub fn unknown(bounds: UtcInterval, origin: AutomaticOrigin) -> Self {
        Self {
            bounds,
            segments: vec![ActivitySegment::new(bounds, ActivityState::Unknown, origin)],
        }
    }

    pub fn bounds(&self) -> UtcInterval {
        self.bounds
    }

    pub fn segments(&self) -> &[ActivitySegment] {
        &self.segments
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SessionState {
    Unknown,
    Locked,
    Unlocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerState {
    Unknown,
    Active,
    Suspended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrackingState {
    Running,
    Paused,
    Finished,
}

/// Idempotency key of a recorded command or observation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationKey(String);

impl OperationKey {
    pub fn new(value: impl Into<String>) -> Result<Self, BlankIdentifier> {
        non_blank(value.into()).map(|value| Self(value.trim().to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for OperationKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorkdayTransitionKind {
    SessionObserved(SessionState),
    PowerObserved(PowerState),
    TrackingChanged(TrackingState),
}

/// A lifecycle change as observed or commanded, before the store assigns it an identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WorkdayTransition {
    occurred_at: Timestamp,
    time_zone_id: String,
    kind: WorkdayTransitionKind,
}

impl WorkdayTransition {
    pub fn new(
        occurred_at: Timestamp,
        time_zone_id: impl Into<String>,
        kind: WorkdayTransitionKind,
    ) -> Result<Self, BlankIdentifier> {
        let time_zone_id = non_blank(time_zone_id.into())?;
        Ok(Self {
            occurred_at,
            time_zone_id: time_zone_id.trim().to_owned(),
            kind,
        })
    }

    pub fn occurred_at(&self) -> Timestamp {
        self.occurred_at
    }

    pub fn time_zone_id(&self) -> &str {
        &self.time_zone_id
    }

    pub fn kind(&self) -> WorkdayTransitionKind {
        self.kind
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkdayEventId(pub i64);

/// A stored transition of one state lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded<S> {
    pub id: WorkdayEventId,
    pub occurred_at: Timestamp,
    pub state: S,
    pub origin: AutomaticOrigin,
}

impl<S> Recorded<S> {
    pub fn new(
        id: WorkdayEventId,
        occurred_at: Timestamp,
        state: S,
        origin: AutomaticOrigin,
    ) -> Self {
        Self {
            id,
            occurred_at,
            state,
            origin,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordedTransition {
    Session(Recorded<SessionState>),
    Power(Recorded<PowerState>),
    Tracking(Recorded<TrackingState>),
}

impl RecordedTransition {
    pub fn id(&self) -> WorkdayEventId {
        match self {
            Self::Session(recorded) => recorded.id,
            Self::Power(recorded) => recorded.id,
            Self::Tracking(recorded) => recorded.id,
        }
    }

    pub fn occurred_at(&self) -> Timestamp {
        match self {
            Self::Session(recorded) => recorded.occurred_at,
            Self::Power(recorded) => recorded.occurred_at,
            Self::Tracking(recorded) => recorded.occurred_at,
        }
    }

    pub fn origin(&self) -> &AutomaticOrigin {
        match self {
            Self::Session(recorded) => &recorded.origin,
            Self::Power(recorded) => &recorded.origin,
            Self::Tracking(recorded) => &recorded.origin,
        }
    }

    /// Unknown observations sort first among simultaneous transitions, so that a decisive state
    /// recorded at the same instant wins.
    fn priority(&self) -> u8 {
        match self {
            Self::Session(Recorded {
                state: SessionState::Unknown,
                ..
            })
            | Self::Power(Recorded {
                state: PowerState::Unknown,
                ..
            }) => 0,
            _ => 1,
        }
    }
}

impl From<Recorded<SessionState>> for RecordedTransition {
    fn from(recorded: Recorded<SessionState>) -> Self {
        Self::Session(recorded)
    }
}

impl From<Recorded<PowerState>> for RecordedTransition {
    fn from(recorded: Recorded<PowerState>) -> Self {
        Self::Power(recorded)
    }
}

impl From<Recorded<TrackingState>> for RecordedTransition {
    fn from(recorded: Recorded<TrackingState>) -> Self {
        Self::Tracking(recorded)
    }
}

/// The last transition of each lane before the workday window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkdayStateAnchors {
    pub session: Option<Recorded<SessionState>>,
    pub power: Option<Recorded<PowerState>>,
    pub tracking: Option<Recorded<TrackingState>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkdayEventHistory {
    before_window: WorkdayStateAnchors,
    in_window: Vec<RecordedTransition>,
}

impl WorkdayEventHistory {
    pub fn new(
        before_window: WorkdayStateAnchors,
        in_window: impl IntoIterator<Item = RecordedTransition>,
    ) -> Self {
        let mut in_window: Vec<_> = in_window.into_iter().collect();
        in_window.sort_by_key(|transition| {
            (
                transition.occurred_at(),
                transition.priority(),
                transition.id(),
            )
        });
        Self {
            before_window,
            in_window,
        }
    }

    pub fn before_window(&self) -> &WorkdayStateAnchors {
        &self.before_window
    }

    pub fn in_window(&self) -> &[RecordedTransition] {
        &self.in_window
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NonPositiveReviewThreshold;

impl std::fmt::Display for NonPositiveReviewThreshold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the long-lock review threshold must be positive")
    }
}

impl std::error::Error for NonPositiveReviewThreshold {}

/// The hours of a recurring appointment on the accounted day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppointmentWindow {
    /// The settings' identifier, which holds no colon.
    pub appointment_id: String,
    pub interval: UtcInterval,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountingPolicy {
    lunch: UtcInterval,
    long_lock_review_threshold: SignedDuration,
    appointments: Vec<AppointmentWindow>,
}

impl AccountingPolicy {
    pub fn new(
        lunch: UtcInterval,
        long_lock_review_threshold: SignedDuration,
    ) -> Result<Self, NonPositiveReviewThreshold> {
        if !long_lock_review_threshold.is_positive() {
            return Err(NonPositiveReviewThreshold);
        }
        Ok(Self {
            lunch,
            long_lock_review_threshold,
            appointments: Vec::new(),
        })
    }

    pub fn with_appointments(self, appointments: Vec<AppointmentWindow>) -> Self {
        Self {
            appointments,
            ..self
        }
    }

    pub fn lunch(&self) -> UtcInterval {
        self.lunch
    }

    pub fn long_lock_review_threshold(&self) -> SignedDuration {
        self.long_lock_review_threshold
    }

    pub fn appointments(&self) -> &[AppointmentWindow] {
        &self.appointments
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidAccountingInput {
    EmptyWindow,
    AnchorNotBeforeWindow,
    TransitionOutsideWindow,
}

impl std::fmt::Display for InvalidAccountingInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::EmptyWindow => "the workday window must have a positive duration",
            Self::AnchorNotBeforeWindow => {
                "a pre-window anchor must occur before the workday window"
            }
            Self::TransitionOutsideWindow => {
                "in-window transitions must lie within the workday window"
            }
        })
    }
}

impl std::error::Error for InvalidAccountingInput {}

/// Everything the engine needs to account one workday up to `now`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountingInput {
    window: WorkdayWindow,
    now: Timestamp,
    policy: AccountingPolicy,
    activity: ActivityCoverage,
    events: WorkdayEventHistory,
    adjustments: Vec<ManualAdjustment>,
}

impl AccountingInput {
    pub fn new(
        window: WorkdayWindow,
        now: Timestamp,
        policy: AccountingPolicy,
        activity: ActivityCoverage,
        events: WorkdayEventHistory,
        adjustments: Vec<ManualAdjustment>,
    ) -> Result<Self, InvalidAccountingInput> {
        if window.end <= window.start {
            return Err(InvalidAccountingInput::EmptyWindow);
        }
        let anchors = events.before_window();
        let anchor_times = [
            anchors.session.as_ref().map(|anchor| anchor.occurred_at),
            anchors.power.as_ref().map(|anchor| anchor.occurred_at),
            anchors.tracking.as_ref().map(|anchor| anchor.occurred_at),
        ];
        if anchor_times
            .into_iter()
            .flatten()
            .any(|at| at >= window.start)
        {
            return Err(InvalidAccountingInput::AnchorNotBeforeWindow);
        }
        if events.in_window().iter().any(|transition| {
            transition.occurred_at() < window.start || transition.occurred_at() > window.end
        }) {
            return Err(InvalidAccountingInput::TransitionOutsideWindow);
        }
        Ok(Self {
            window,
            now,
            policy,
            activity,
            events,
            adjustments,
        })
    }

    pub fn window(&self) -> &WorkdayWindow {
        &self.window
    }

    pub fn now(&self) -> Timestamp {
        self.now
    }

    pub fn policy(&self) -> &AccountingPolicy {
        &self.policy
    }

    pub fn activity(&self) -> &ActivityCoverage {
        &self.activity
    }

    pub fn events(&self) -> &WorkdayEventHistory {
        &self.events
    }

    pub fn adjustments(&self) -> &[ManualAdjustment] {
        &self.adjustments
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minute(minute: i64) -> Timestamp {
        Timestamp::from_second(1_788_000_000 + minute * 60).unwrap()
    }

    fn interval(start: i64, end: i64) -> UtcInterval {
        UtcInterval::new(minute(start), minute(end)).unwrap()
    }

    fn origin(id: &str) -> AutomaticOrigin {
        AutomaticOrigin::new(id).unwrap()
    }

    #[test]
    fn blank_source_identifiers_are_rejected() {
        assert_eq!(AutomaticOrigin::new("  "), Err(BlankSourceId));
    }

    #[test]
    fn coverage_clips_drops_and_sorts_segments() {
        let coverage = ActivityCoverage::new(
            interval(0, 60),
            [
                ActivitySegment::new(interval(30, 90), ActivityState::Afk, origin("b")),
                ActivitySegment::new(interval(70, 80), ActivityState::Active, origin("a")),
                ActivitySegment::new(interval(-10, 30), ActivityState::Active, origin("b")),
                ActivitySegment::new(interval(0, 30), ActivityState::Active, origin("a")),
            ],
        );

        assert_eq!(
            coverage.segments(),
            [
                ActivitySegment::new(interval(0, 30), ActivityState::Active, origin("a")),
                ActivitySegment::new(interval(0, 30), ActivityState::Active, origin("b")),
                ActivitySegment::new(interval(30, 60), ActivityState::Afk, origin("b")),
            ]
        );
    }
}
