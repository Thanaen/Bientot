//! The built-in activity source, ported from `Services/BuiltInActivitySource.cs` and
//! `Activity/LastInputPort.cs`. It samples the last-input time, records what it settles as
//! activity spans, and answers reads with settled spans only.
//! See `specs/001-core-workday-indicator/contracts/built-in-activity-source.md`.

#[cfg(windows)]
mod native;

use jiff::{SignedDuration, Timestamp};
use worktime_domain::accounting::{
    ActivityCoverage, ActivitySegment, ActivityState, AutomaticOrigin,
};
use worktime_domain::configuration::{BuiltInActivitySettings, WorkTimeSettings};
use worktime_domain::intervals::UtcInterval;
use worktime_domain::sources::{
    ActivitySourceKind, IdleObservation, IdleSpan, IdleState, IdleStateTracker, LastInputReading,
    SourceFreshness, SourceHealth, SourceQuality,
};
use worktime_domain::time::Clock;
use worktime_store::activity::{ActivityError, ActivitySpanId, ActivitySpanRepository};

use crate::PowerState;

#[cfg(windows)]
pub use native::SystemLastInput;

/// Reads when the current session last received keyboard or mouse input, and nothing else about
/// it.
pub trait LastInputProbe {
    /// `None` when the system query fails.
    fn read(&self) -> Option<LastInputReading>;
}

/// The reading for a `GetTickCount64` value and a `GetLastInputInfo` tick. Only the low 32 bits
/// of the tick count are kept, so the idle duration survives the 49.7-day wraparound of the
/// input tick.
pub fn reading_from_ticks(tick_count64: u64, last_input_tick: u32) -> LastInputReading {
    LastInputReading {
        tick_count: tick_count64 as u32,
        last_input_tick,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdleThresholdOutOfRange;

impl std::fmt::Display for IdleThresholdOutOfRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the idle threshold must be from 30 seconds to 30 minutes")
    }
}

impl std::error::Error for IdleThresholdOutOfRange {}

#[derive(Debug)]
pub enum BuiltInSourceError {
    Storage(ActivityError),
    IdleThreshold(IdleThresholdOutOfRange),
}

impl std::fmt::Display for BuiltInSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(f),
            Self::IdleThreshold(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for BuiltInSourceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::IdleThreshold(error) => Some(error),
        }
    }
}

impl From<ActivityError> for BuiltInSourceError {
    fn from(error: ActivityError) -> Self {
        Self::Storage(error)
    }
}

impl From<IdleThresholdOutOfRange> for BuiltInSourceError {
    fn from(error: IdleThresholdOutOfRange) -> Self {
        Self::IdleThreshold(error)
    }
}

/// What a read of the built-in source returns: the settled activity in the requested interval
/// and how the source is doing. The .NET `ActivityReadResult.Snapshot` also names an
/// ActivityWatch bucket, always null for this source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivitySnapshot {
    pub coverage: ActivityCoverage,
    pub health: SourceHealth,
}

/// The built-in source. The host owns it on one thread: it calls [`Self::poll`] every
/// [`IdleStateTracker::DEFAULT_POLL_INTERVAL`] and [`Self::observe_power`] on each power
/// notification, and reads with [`Self::read`].
pub struct BuiltInActivitySource<'a> {
    input: &'a dyn LastInputProbe,
    spans: ActivitySpanRepository<'a>,
    clock: &'a dyn Clock,
    tracker: IdleStateTracker,
    idle_threshold: SignedDuration,
    open_id: Option<ActivitySpanId>,
    stored_settled_through: Timestamp,
    last_successful_sample: Option<Timestamp>,
    last_query_failed: bool,
    suspended: bool,
}

impl<'a> BuiltInActivitySource<'a> {
    pub const SOURCE_ID: &'static str = "built-in";
    /// How far the stored settled boundary of the open span may lag behind the tracker's, so
    /// that a crash loses at most this much settled activity without a write every sample.
    const STORED_SETTLEMENT_LAG: SignedDuration = SignedDuration::from_mins(1);

    pub fn new(
        input: &'a dyn LastInputProbe,
        spans: ActivitySpanRepository<'a>,
        clock: &'a dyn Clock,
    ) -> Self {
        Self {
            input,
            spans,
            clock,
            tracker: IdleStateTracker::default(),
            idle_threshold: BuiltInActivitySettings::DEFAULT_IDLE_THRESHOLD,
            open_id: None,
            stored_settled_through: Timestamp::UNIX_EPOCH,
            last_successful_sample: None,
            last_query_failed: false,
            suspended: false,
        }
    }

    pub fn idle_threshold(&self) -> SignedDuration {
        self.idle_threshold
    }

    /// Applies to samples taken from now on; stored spans keep the boundaries they were settled
    /// with.
    pub fn set_idle_threshold(
        &mut self,
        threshold: SignedDuration,
    ) -> Result<(), IdleThresholdOutOfRange> {
        if !(BuiltInActivitySettings::MINIMUM_IDLE_THRESHOLD
            ..=BuiltInActivitySettings::MAXIMUM_IDLE_THRESHOLD)
            .contains(&threshold)
        {
            return Err(IdleThresholdOutOfRange);
        }
        self.idle_threshold = threshold;
        Ok(())
    }

    /// The host's tick: samples with the saved threshold while the built-in source is selected,
    /// and otherwise ends observation so that switching back starts a new span. Nothing is
    /// sampled between a suspend notification and the next resume.
    pub fn poll(&mut self, settings: &WorkTimeSettings) -> Result<(), BuiltInSourceError> {
        if settings.activity_source != Some(ActivitySourceKind::BuiltIn) {
            self.end_observation()?;
            return Ok(());
        }
        self.set_idle_threshold(settings.built_in_activity.idle_threshold)?;
        self.sample()?;
        Ok(())
    }

    /// The host's power notifications: suspend stores the open span's settled boundary before
    /// the computer sleeps and stops sampling until resume, and resume samples at once so that a
    /// new span starts at resume rather than at the next tick, however short the sleep.
    pub fn observe_power(
        &mut self,
        state: PowerState,
        settings: &WorkTimeSettings,
    ) -> Result<(), BuiltInSourceError> {
        self.suspended = state == PowerState::Suspended;
        self.end_observation()?;
        if state != PowerState::Suspended {
            self.poll(settings)?;
        }
        Ok(())
    }

    /// Takes one sample, unless the computer is suspended.
    pub fn sample(&mut self) -> Result<(), ActivityError> {
        if self.suspended {
            return Ok(());
        }
        let now = self.clock.now();
        let Some(reading) = self.input.read() else {
            self.last_query_failed = true;
            return self.end_observation();
        };
        self.last_query_failed = false;
        self.last_successful_sample = Some(now);
        let observation = self.tracker.observe(now, reading, self.idle_threshold);
        self.record(observation).inspect_err(|_| {
            self.tracker.interrupt();
            self.open_id = None;
        })
    }

    /// Stores the settled boundary of the open span and ends uninterrupted observation: on clean
    /// shutdown, suspend, resume, or a switch to another source. The next sample starts a new
    /// span.
    pub fn end_observation(&mut self) -> Result<(), ActivityError> {
        let ended = self.tracker.interrupt();
        let ended_id = self.open_id.take();
        if let (Some(ended), Some(id)) = (ended, ended_id) {
            self.spans.advance_settled(id, ended.settled_through())?;
        }
        Ok(())
    }

    pub fn read(&self, interval: UtcInterval) -> Result<ActivitySnapshot, ActivityError> {
        let stored = self.spans.read_settled(interval.start(), interval.end())?;
        let open = self.open_id.and(self.tracker.open());
        let origin = AutomaticOrigin::new(Self::SOURCE_ID).expect("the source id is not blank");
        let settled = stored
            .into_iter()
            .filter(|span| Some(span.id) != self.open_id)
            .map(|span| span.span)
            .chain(open)
            .filter_map(|span| {
                let interval = UtcInterval::new(span.start(), span.settled_through()).ok()?;
                let state = match span.state() {
                    IdleState::Active => ActivityState::Active,
                    IdleState::Idle => ActivityState::Afk,
                };
                Some(ActivitySegment::new(interval, state, origin.clone()))
            });
        Ok(ActivitySnapshot {
            coverage: ActivityCoverage::new(interval, settled),
            health: self.health(self.clock.now()),
        })
    }

    fn record(&mut self, observation: IdleObservation) -> Result<(), ActivityError> {
        match observation {
            IdleObservation::Started { open, abandoned } => {
                if let (Some(abandoned), Some(id)) = (abandoned, self.open_id) {
                    self.spans
                        .advance_settled(id, abandoned.settled_through())?;
                }
                self.append(open, None)?;
            }
            IdleObservation::Transitioned { open, .. } => self.append(open, self.open_id)?,
            IdleObservation::Continued { open } => {
                if let Some(id) = self.open_id
                    && open
                        .settled_through()
                        .duration_since(self.stored_settled_through)
                        > Self::STORED_SETTLEMENT_LAG
                {
                    self.spans.advance_settled(id, open.settled_through())?;
                    self.stored_settled_through = open.settled_through();
                }
            }
        }
        Ok(())
    }

    fn append(
        &mut self,
        open: IdleSpan,
        closes: Option<ActivitySpanId>,
    ) -> Result<(), ActivityError> {
        self.open_id = Some(self.spans.append(open, closes)?);
        self.stored_settled_through = open.settled_through();
        Ok(())
    }

    fn health(&self, now: Timestamp) -> SourceHealth {
        let (freshness, reason) = if self.last_query_failed {
            (SourceFreshness::Unavailable, "input-query-failed")
        } else {
            match self.last_successful_sample {
                Some(last)
                    if now.duration_since(last) <= IdleStateTracker::DEFAULT_POLL_INTERVAL * 2 =>
                {
                    return SourceHealth::healthy(Self::SOURCE_ID, last);
                }
                _ => (SourceFreshness::Stale, "not-sampling"),
            }
        };
        SourceHealth::degraded(
            Self::SOURCE_ID,
            freshness,
            SourceQuality::Incomplete,
            reason,
            self.last_successful_sample,
        )
        .expect("an incomplete source with a reason is degraded")
    }
}

#[cfg(test)]
mod tests;
