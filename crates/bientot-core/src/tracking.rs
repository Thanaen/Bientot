//! Pause, resume and finish, ported from `Services/TrackingControlService.cs`. Each command is a
//! transition of the user lane of the ledger, idempotent under its operation key. A finished day
//! stays finished until the next working day starts; a pause lasts until the user resumes.

use bientot_domain::accounting::{
    OperationKey, RecordedTransition, TrackingState, WorkdayTransition, WorkdayTransitionKind,
};
use bientot_domain::time::{CalendarError, Clock, LocalWorkday, TimeZoneProvider, WorkdayCalendar};
use bientot_store::sessions::{
    RecordTransitionOutcome, RecordTransitionResult, SessionEventError, SessionEventRepository,
};
use jiff::civil::{Date, Weekday};

const FINISHED_RESET_KEY_PREFIX: &str = "tracking-finished-reset";

#[derive(Debug)]
pub enum TrackingError {
    Events(SessionEventError),
    Calendar(CalendarError),
}

impl std::fmt::Display for TrackingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Events(error) => error.fmt(f),
            Self::Calendar(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for TrackingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Events(error) => Some(error),
            Self::Calendar(error) => Some(error),
        }
    }
}

impl From<SessionEventError> for TrackingError {
    fn from(error: SessionEventError) -> Self {
        Self::Events(error)
    }
}

impl From<CalendarError> for TrackingError {
    fn from(error: CalendarError) -> Self {
        Self::Calendar(error)
    }
}

pub struct TrackingControl<'a> {
    events: SessionEventRepository<'a>,
    clock: &'a dyn Clock,
    calendar: WorkdayCalendar<&'a dyn TimeZoneProvider>,
    request_refresh: &'a dyn Fn(),
}

impl<'a> TrackingControl<'a> {
    /// `request_refresh` runs after a command changed the ledger. It only asks for a refresh and
    /// cannot fail: the command is recorded whatever the refresh then does.
    pub fn new(
        events: SessionEventRepository<'a>,
        clock: &'a dyn Clock,
        zones: &'a dyn TimeZoneProvider,
        request_refresh: &'a dyn Fn(),
    ) -> Self {
        Self {
            events,
            clock,
            calendar: WorkdayCalendar::new(zones),
            request_refresh,
        }
    }

    pub fn pause(&self, key: OperationKey) -> Result<RecordTransitionResult, TrackingError> {
        self.record_now(key, TrackingState::Paused)
    }

    pub fn resume(&self, key: OperationKey) -> Result<RecordTransitionResult, TrackingError> {
        self.record_now(key, TrackingState::Running)
    }

    pub fn finish(&self, key: OperationKey) -> Result<RecordTransitionResult, TrackingError> {
        self.record_now(key, TrackingState::Finished)
    }

    /// Resets a finished day that the current workday has outlived, then returns the tracking
    /// state in effect now.
    pub fn reconcile_current_state(&self) -> Result<TrackingState, TrackingError> {
        let now = self.clock.now();
        let workday = self.calendar.workday(now);
        self.reconcile(&workday)?;

        let history = self
            .events
            .read_for_accounting(&self.calendar.window(&workday)?)?;
        let anchored = history
            .before_window()
            .tracking
            .as_ref()
            .map_or(TrackingState::Running, |anchor| anchor.state);
        Ok(history
            .in_window()
            .iter()
            .rev()
            .find_map(|transition| match transition {
                RecordedTransition::Tracking(recorded) if recorded.occurred_at <= now => {
                    Some(recorded.state)
                }
                _ => None,
            })
            .unwrap_or(anchored))
    }

    /// Records the running state at the start of the first working day after a finish, once
    /// `current` has reached that day, under a key derived from the finish so that restarts and
    /// concurrent calls record it once. A pause is never reset. `None` when nothing was due.
    pub fn reconcile(
        &self,
        current: &LocalWorkday,
    ) -> Result<Option<RecordTransitionResult>, TrackingError> {
        if !current.is_working_day() {
            return Ok(None);
        }
        let history = self
            .events
            .read_for_accounting(&self.calendar.window(current)?)?;
        let Some(finished) = history
            .before_window()
            .tracking
            .as_ref()
            .filter(|anchor| anchor.state == TrackingState::Finished)
        else {
            return Ok(None);
        };

        let finished_on = finished
            .occurred_at
            .to_zoned(
                self.calendar
                    .zones()
                    .get(&current.time_zone_id)
                    .map_err(CalendarError::from)?,
            )
            .date();
        let reset = LocalWorkday::new(
            first_working_day_after(finished_on),
            current.time_zone_id.clone(),
        );
        if reset.date > current.date {
            return Ok(None);
        }
        let key = OperationKey::new(format!(
            "{FINISHED_RESET_KEY_PREFIX}:{}:{}:{}",
            finished.id.0, reset.date, reset.time_zone_id
        ))
        .expect("the key starts with its prefix");
        let transition = WorkdayTransition::new(
            self.calendar.window(&reset)?.start,
            &reset.time_zone_id,
            WorkdayTransitionKind::TrackingChanged(TrackingState::Running),
        )
        .expect("a workday's time zone is named");
        Ok(Some(self.events.record(key, transition)?))
    }

    fn record_now(
        &self,
        key: OperationKey,
        state: TrackingState,
    ) -> Result<RecordTransitionResult, TrackingError> {
        let now = self.clock.now();
        let workday = self.calendar.workday(now);
        let transition = WorkdayTransition::new(
            now,
            workday.time_zone_id,
            WorkdayTransitionKind::TrackingChanged(state),
        )
        .expect("the local time zone is named");
        let result = match self.events.record(key, transition) {
            Err(SessionEventError::OperationConflict(key)) => return self.retried(key, state),
            result => result?,
        };
        if result.outcome == RecordTransitionOutcome::Inserted {
            (self.request_refresh)();
        }
        Ok(result)
    }

    /// A retry stamps a later instant than the command it repeats, so the store sees a different
    /// transition under the same key. The key still names the same command when it recorded the
    /// same tracking state.
    fn retried(
        &self,
        key: OperationKey,
        state: TrackingState,
    ) -> Result<RecordTransitionResult, TrackingError> {
        match self.events.find(&key)? {
            Some(RecordedTransition::Tracking(recorded)) if recorded.state == state => {
                Ok(RecordTransitionResult {
                    outcome: RecordTransitionOutcome::AlreadyRecorded,
                    authoritative_id: recorded.id,
                })
            }
            _ => Err(SessionEventError::OperationConflict(key).into()),
        }
    }
}

fn first_working_day_after(date: Date) -> Date {
    let mut day = date
        .tomorrow()
        .expect("workdays are far from the calendar's end");
    while matches!(day.weekday(), Weekday::Saturday | Weekday::Sunday) {
        day = day
            .tomorrow()
            .expect("workdays are far from the calendar's end");
    }
    day
}

#[cfg(test)]
mod tests;
