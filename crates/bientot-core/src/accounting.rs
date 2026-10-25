//! The calculation of one workday from the activity source and the ledger, ported from
//! `Services/WorkdayAccountingService.cs`. Every refresh, recalculation, correction and review
//! starts from it.

use bientot_domain::accounting::{
    self, AccountingInput, AccountingPolicy, ActivityCoverage, AppointmentWindow, AutomaticOrigin,
    Recorded, RecordedTransition, TrackingState, WorkdayEventHistory, WorkdayEventId,
    WorkdayStateAnchors,
};
use bientot_domain::adjustments::AdjustmentValidationError;
use bientot_domain::configuration::Settings;
use bientot_domain::intervals::UtcInterval;
use bientot_domain::sources::{
    ActivitySourceKind, SourceDiagnostics, SourceFreshness, SourceHealth, SourceIntervalKind,
    SourceQuality,
};
use bientot_domain::summaries::{self, DailyAccountingCalculation, DailySummary};
use bientot_domain::time::{
    CalendarError, Clock, FixedTimeZoneProvider, LocalWorkday, TimeZoneProvider, UnknownTimeZone,
    WorkdayCalendar, WorkdayWindow,
};
use bientot_store::activity::ActivityError;
use bientot_store::adjustments::{ManualAdjustmentError, ManualAdjustmentRepository};
use bientot_store::sessions::{SessionEventError, SessionEventRepository};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use crate::routing::NO_SOURCE_ID;
use crate::sources::{ActivityReadResult, ActivitySource, NormalizedActivityEvidence, normalize};

/// The source the diagnostics name when nothing was read: the .NET identifier, kept for the
/// fingerprints.
const UNREAD_SOURCE_ID: &str = "activitywatch";
const NON_WORKING_DAY_ORIGIN: &str = "schedule:non-working-day";

/// The settings, local zone and instant one calculation runs with, read once so that the days
/// of one refresh agree.
#[derive(Clone, Debug, PartialEq)]
pub struct CalculationContext {
    pub settings: Settings,
    pub time_zone_id: String,
    pub time_zone: TimeZone,
    pub observed_at: Timestamp,
}

impl CalculationContext {
    pub(crate) fn calendar(&self) -> WorkdayCalendar<FixedTimeZoneProvider> {
        WorkdayCalendar::new(FixedTimeZoneProvider::new(
            self.time_zone_id.clone(),
            self.time_zone.clone(),
        ))
    }
}

/// What the activity read of a calculation returned, which decides whether its evidence can
/// replace what the ledger already holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActivityReadDisposition {
    Snapshot,
    Unavailable,
    /// The day had not started at the observation instant, so nothing was read.
    NotRequested,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkdayCalculation {
    pub summary_input: DailyAccountingCalculation,
    pub activity_read: ActivityReadDisposition,
}

impl WorkdayCalculation {
    /// Whether the source answered, however degraded its answer.
    pub fn has_usable_source_snapshot(&self) -> bool {
        self.activity_read == ActivityReadDisposition::Snapshot
    }

    /// Whether the source answered and reported no operational issue.
    pub fn has_complete_source_snapshot(&self) -> bool {
        self.has_usable_source_snapshot()
            && !self
                .summary_input
                .source_diagnostics()
                .has_operational_issue()
    }

    /// Whether a past day can be replayed from this calculation: the source answered completely
    /// and knew something about the day, rather than leaving all of it unresolved, so a closed
    /// week is never rewritten from an empty read.
    pub fn has_historical_replay_evidence(&self) -> bool {
        let diagnostics = self.summary_input.source_diagnostics();
        self.has_complete_source_snapshot()
            && !diagnostics.intervals().iter().any(|interval| {
                interval.kind() == SourceIntervalKind::UnresolvedCoverage
                    && interval.interval() == diagnostics.request_bounds()
            })
    }

    /// Whether no source was selected for any of the day, so that no later read can know more
    /// than this calculation: the day stays unknown.
    pub fn predates_first_selection(&self) -> bool {
        self.activity_read == ActivityReadDisposition::Unavailable
            && self
                .summary_input
                .source_diagnostics()
                .operation()
                .source_id
                == NO_SOURCE_ID
    }
}

#[derive(Debug)]
pub enum AccountingError {
    /// The day asked for is after the current local day.
    FutureWorkday,
    Calendar(CalendarError),
    Events(SessionEventError),
    Adjustments(ManualAdjustmentError),
    Activity(ActivityError),
    /// The stored adjustments no longer apply to the day's timeline.
    InvalidAdjustment(AdjustmentValidationError),
}

impl std::fmt::Display for AccountingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FutureWorkday => {
                f.write_str("A workday calculation cannot target a future local date.")
            }
            Self::Calendar(error) => error.fmt(f),
            Self::Events(error) => error.fmt(f),
            Self::Adjustments(error) => error.fmt(f),
            Self::Activity(error) => error.fmt(f),
            Self::InvalidAdjustment(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for AccountingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FutureWorkday => None,
            Self::Calendar(error) => Some(error),
            Self::Events(error) => Some(error),
            Self::Adjustments(error) => Some(error),
            Self::Activity(error) => Some(error),
            Self::InvalidAdjustment(error) => Some(error),
        }
    }
}

impl From<CalendarError> for AccountingError {
    fn from(error: CalendarError) -> Self {
        Self::Calendar(error)
    }
}

impl From<SessionEventError> for AccountingError {
    fn from(error: SessionEventError) -> Self {
        Self::Events(error)
    }
}

impl From<ManualAdjustmentError> for AccountingError {
    fn from(error: ManualAdjustmentError) -> Self {
        Self::Adjustments(error)
    }
}

impl From<ActivityError> for AccountingError {
    fn from(error: ActivityError) -> Self {
        Self::Activity(error)
    }
}

impl From<AdjustmentValidationError> for AccountingError {
    fn from(error: AdjustmentValidationError) -> Self {
        Self::InvalidAdjustment(error)
    }
}

/// Calculates workdays from the activity source, the recorded transitions and the manual
/// adjustments. `settings` is read once per calculation, so a change applies from the next one.
pub struct WorkdayAccounting<'a> {
    source: &'a dyn ActivitySource,
    events: SessionEventRepository<'a>,
    adjustments: ManualAdjustmentRepository<'a>,
    clock: &'a dyn Clock,
    zones: &'a dyn TimeZoneProvider,
    settings: &'a dyn Fn() -> Settings,
}

impl<'a> WorkdayAccounting<'a> {
    pub fn new(
        source: &'a dyn ActivitySource,
        events: SessionEventRepository<'a>,
        adjustments: ManualAdjustmentRepository<'a>,
        clock: &'a dyn Clock,
        zones: &'a dyn TimeZoneProvider,
        settings: &'a dyn Fn() -> Settings,
    ) -> Self {
        Self {
            source,
            events,
            adjustments,
            clock,
            zones,
            settings,
        }
    }

    /// Today's summary against the nominal target, without weekly credit.
    pub fn calculate_current(&self) -> Result<DailySummary, AccountingError> {
        let context = self.capture_context();
        let today = context.calendar().workday(context.observed_at).date;
        let calculation = self.calculate(today, &context)?;
        Ok(summaries::build_nominal(&calculation.summary_input)
            .expect("a summary of a domain calculation is consistent"))
    }

    /// The zone a stored workday names, for replaying it where it was recorded.
    pub fn resolve_time_zone(&self, time_zone_id: &str) -> Result<TimeZone, UnknownTimeZone> {
        self.zones.get(time_zone_id)
    }

    pub fn capture_context(&self) -> CalculationContext {
        let (time_zone_id, time_zone) = self.zones.local();
        CalculationContext {
            settings: (self.settings)(),
            time_zone_id,
            time_zone,
            observed_at: self.clock.now(),
        }
    }

    /// Calculates `date` in the context's zone as observed at the context's instant: a past day
    /// is accounted through its end, the current day through the observation instant.
    pub fn calculate(
        &self,
        date: Date,
        context: &CalculationContext,
    ) -> Result<WorkdayCalculation, AccountingError> {
        let calendar = context.calendar();
        let current = calendar.workday(context.observed_at);
        if date > current.date {
            return Err(AccountingError::FutureWorkday);
        }

        let settings = &context.settings;
        let workday = LocalWorkday::new(date, context.time_zone_id.clone());
        let window = calendar.window(&workday)?;
        let projection_anchor = if date < current.date {
            window.end
        } else {
            context.observed_at.clamp(window.start, window.end)
        };
        let started = projection_anchor > window.start;
        let activity_bounds = UtcInterval::new(
            window.start,
            if started {
                projection_anchor
            } else {
                window.end
            },
        )
        .expect("a workday window has a positive duration");

        let events = self.events.read_for_accounting(&window)?;
        let adjustments = if workday.is_working_day() {
            self.adjustments.read_for_workday(&workday)?
        } else {
            Vec::new()
        };
        let activity = if started {
            Some(self.source.read(activity_bounds)?)
        } else {
            None
        };

        let evidence = match &activity {
            Some(result) => normalize(
                activity_bounds,
                context.observed_at,
                result,
                pending_horizon(settings),
            ),
            None => NormalizedActivityEvidence {
                coverage: ActivityCoverage::new(activity_bounds, []),
                diagnostics: unread_diagnostics(activity_bounds, context.observed_at),
            },
        };
        let lunch = calendar.interval(&workday, settings.lunch_start, settings.lunch_end)?;
        let appointments = settings
            .recurring_appointments
            .iter()
            .filter(|appointment| appointment.falls_on(date))
            .map(|appointment| {
                Ok(AppointmentWindow {
                    appointment_id: appointment.id.clone(),
                    interval: calendar.interval(&workday, appointment.start, appointment.end)?,
                })
            })
            .collect::<Result<_, AccountingError>>()?;
        let policy = AccountingPolicy::new(lunch, settings.long_lock_review_threshold)
            .expect("validated settings have a positive review threshold")
            .with_appointments(appointments);
        let input = AccountingInput::new(
            window.clone(),
            projection_anchor,
            policy,
            evidence.coverage,
            apply_schedule(&workday, &window, events),
            adjustments,
        )
        .expect("the ledger returns the transitions of the window it was asked for");
        let summary_input = DailyAccountingCalculation::new(
            accounting::calculate(&input)?,
            settings.nominal_daily_target,
            lunch,
            calendar.instant(&workday, settings.preferred_latest_departure)?,
            settings.orange_warning_margin,
            evidence.diagnostics,
        )
        .expect("validated settings have a positive target and a non-negative margin");
        Ok(WorkdayCalculation {
            summary_input,
            activity_read: match activity {
                Some(ActivityReadResult::Snapshot { .. }) => ActivityReadDisposition::Snapshot,
                Some(ActivityReadResult::Unavailable { .. }) => {
                    ActivityReadDisposition::Unavailable
                }
                None => ActivityReadDisposition::NotRequested,
            },
        })
    }
}

/// The uncovered live tail settles under the rules of the source selected now, which answers for
/// it.
fn pending_horizon(settings: &Settings) -> SignedDuration {
    if settings.activity_source == Some(ActivitySourceKind::BuiltIn) {
        settings.built_in_activity.pending_horizon()
    } else {
        settings.activity_watch.settlement.horizon()
    }
}

/// A weekend is finished from its first instant whatever the user recorded, so that its activity
/// is never credited.
fn apply_schedule(
    workday: &LocalWorkday,
    window: &WorkdayWindow,
    history: WorkdayEventHistory,
) -> WorkdayEventHistory {
    if workday.is_working_day() {
        return history;
    }
    let schedule_finished = Recorded::new(
        WorkdayEventId(i64::MIN),
        window.start,
        TrackingState::Finished,
        AutomaticOrigin::new(NON_WORKING_DAY_ORIGIN).expect("the schedule origin is not blank"),
    );
    let before = history.before_window();
    WorkdayEventHistory::new(
        WorkdayStateAnchors {
            session: before.session.clone(),
            power: before.power.clone(),
            tracking: None,
        },
        history
            .in_window()
            .iter()
            .filter(|transition| !matches!(transition, RecordedTransition::Tracking(_)))
            .cloned()
            .chain([schedule_finished.into()]),
    )
}

/// Diagnostics of a day that had not started: healthy, with nothing to report.
fn unread_diagnostics(bounds: UtcInterval, observed_at: Timestamp) -> SourceDiagnostics {
    let health = SourceHealth {
        source_id: UNREAD_SOURCE_ID.to_owned(),
        freshness: SourceFreshness::Fresh,
        quality: SourceQuality::Complete,
        last_successful_at: None,
        degraded_reason: None,
    };
    SourceDiagnostics::new(health, bounds, observed_at, [])
        .expect("diagnostics without intervals are valid")
}

#[cfg(test)]
mod tests;
