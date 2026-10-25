//! The `--demo` scenarios, ported from `Services/Demo/DemoEnvironment.cs`,
//! `Services/Demo/DemoHistoryWeek.cs`, `Services/Demo/DemoSummaryCalculator.cs` and the scenario
//! seeding in `App.xaml.cs`. A demo is a clock and an activity source fixed on a scenario; the
//! ledger and the services are the real ones, run in the throwaway data directory.
//!
//! Not ported here: the argument parsing and the scenario picker (R6), and
//! `DemoStoreUpdatePort`, which is `bientot_app::updates::DemoStore` beside the `UpdateStore`
//! trait of `bientot-windows`.

use std::cell::Cell;

use bientot_domain::accounting::{
    ActivityCoverage, ActivitySegment, ActivityState, AdjustmentId, AutomaticOrigin, OperationKey,
    PowerState, SessionState, WorkdayTransition, WorkdayTransitionKind,
};
use bientot_domain::adjustments::ManualAdjustment;
use bientot_domain::credit::{DailyCreditInput, calculate_week};
use bientot_domain::intervals::UtcInterval;
use bientot_domain::sources::{SourceFreshness, SourceHealth, SourceQuality};
use bientot_domain::summaries::{self, DailySummary};
use bientot_domain::time::{Clock, LocalWorkday, TimeZoneProvider, WorkdayCalendar};
use bientot_store::activity::ActivityError;
use bientot_store::sessions::{SessionEventError, SessionEventRepository};
use jiff::civil::{Date, Time, Weekday, time};
use jiff::{SignedDuration, Timestamp, ToSpan};

use crate::accounting::{AccountingError, WorkdayAccounting};
use crate::corrections::{CorrectionError, ManualAdjustmentCommands};
use crate::routing::ACTIVITY_WATCH_ID;
use crate::sources::{ActivityReadResult, ActivitySource};
use crate::weekly::{WeeklyError, WeeklyRecalculation, WorkdayRefreshState};

/// The id of the built-in source, `BuiltInActivitySource::SOURCE_ID` in `bientot-windows`, so
/// a demo source problem gets the built-in source's guidance rather than ActivityWatch's. The
/// .NET demo reports "demo", which the presenter treats as ActivityWatch.
const SOURCE_ID: &str = "built-in";
const OFFLINE_REASON: &str = "demo-offline";
const SYNTHETIC_ORIGIN: &str = "demo:synthetic-afk";
const HISTORY_WEEK_ORIGIN: &str = "demo:history-week";
const HISTORY_WEEK_REASON: &str = "demo history fixture";

/// The scenarios in the order of the .NET enum, which is the order of the scenario picker, then
/// [`DemoScenario::ActivityWatchUnavailable`], which the .NET app does not have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DemoScenario {
    OnTrack,
    Warning,
    Late,
    Complete,
    Morning,
    Unavailable,
    Credit,
    HistoryWeek,
    MorningWarning,
    MorningLate,
    ProblemPeriods,
    FragmentedDay,
    /// `Unavailable`, reported by ActivityWatch rather than the built-in source.
    ActivityWatchUnavailable,
}

impl DemoScenario {
    pub const ALL: [Self; 13] = [
        Self::OnTrack,
        Self::Warning,
        Self::Late,
        Self::Complete,
        Self::Morning,
        Self::Unavailable,
        Self::Credit,
        Self::HistoryWeek,
        Self::MorningWarning,
        Self::MorningLate,
        Self::ProblemPeriods,
        Self::FragmentedDay,
        Self::ActivityWatchUnavailable,
    ];

    /// The resource key of the scenario's name in the picker.
    pub fn resource_key(self) -> &'static str {
        match self {
            Self::OnTrack => "DemoOnTrack",
            Self::Warning => "DemoWarning",
            Self::Late => "DemoLate",
            Self::Complete => "DemoComplete",
            Self::Morning => "DemoMorning",
            // « ActivityWatch indisponible », the name of the .NET scenario, whose source the
            // presenter treats as ActivityWatch.
            Self::Unavailable | Self::ActivityWatchUnavailable => "DemoUnavailable",
            Self::Credit => "DemoCredit",
            Self::HistoryWeek => "DemoHistoryWeek",
            Self::MorningWarning => "DemoMorningWarning",
            Self::MorningLate => "DemoMorningLate",
            Self::ProblemPeriods => "DemoProblemPeriods",
            Self::FragmentedDay => "DemoFragmentedDay",
        }
    }

    /// Monday 31 August 2026, Tuesday for the credit and Friday for the stored week.
    fn date(self) -> Date {
        match self {
            Self::Credit => jiff::civil::date(2026, 9, 1),
            Self::HistoryWeek => jiff::civil::date(2026, 9, 4),
            _ => jiff::civil::date(2026, 8, 31),
        }
    }

    fn local_now(self) -> Time {
        match self {
            Self::Complete | Self::HistoryWeek => time(18, 0, 0, 0),
            Self::Morning | Self::MorningWarning | Self::MorningLate => time(11, 0, 0, 0),
            _ => time(16, 0, 0, 0),
        }
    }

    /// The days whose summaries the week view shows go through the weekly recalculation; the
    /// others are one day calculated on its own.
    fn is_stored(self) -> bool {
        matches!(
            self,
            Self::HistoryWeek | Self::ProblemPeriods | Self::FragmentedDay
        )
    }
}

#[derive(Debug)]
pub enum DemoSeedError {
    Events(SessionEventError),
    Correction(CorrectionError),
}

impl std::fmt::Display for DemoSeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Events(error) => error.fmt(f),
            Self::Correction(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for DemoSeedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Events(error) => Some(error),
            Self::Correction(error) => Some(error),
        }
    }
}

impl From<SessionEventError> for DemoSeedError {
    fn from(error: SessionEventError) -> Self {
        Self::Events(error)
    }
}

impl From<CorrectionError> for DemoSeedError {
    fn from(error: CorrectionError) -> Self {
        Self::Correction(error)
    }
}

/// The clock and the activity source of a demo, in the local zone of `zones`. The scenario can
/// change while the services borrow the environment, as the picker changes it.
pub struct DemoEnvironment<Z> {
    zones: Z,
    scenario: Cell<DemoScenario>,
}

impl<Z: TimeZoneProvider> DemoEnvironment<Z> {
    pub fn new(zones: Z, scenario: DemoScenario) -> Self {
        Self {
            zones,
            scenario: Cell::new(scenario),
        }
    }

    pub fn scenario(&self) -> DemoScenario {
        self.scenario.get()
    }

    pub fn set_scenario(&self, scenario: DemoScenario) {
        self.scenario.set(scenario);
    }

    /// Writes the ledger the scenario starts from: the lifecycle events and corrections of the
    /// stored week, the sleep of the problem periods, the sleeps of the fragmented day. Every
    /// write has a fixed key, so seeding again changes nothing. The other scenarios start empty.
    pub fn seed(
        &self,
        events: &SessionEventRepository<'_>,
        corrections: &ManualAdjustmentCommands<'_>,
    ) -> Result<(), DemoSeedError> {
        let today = self.workday();
        for (key, at, kind) in self.seeded_transitions(&today) {
            events.record(
                OperationKey::new(key).expect("the demo keys are not blank"),
                WorkdayTransition::new(at, today.time_zone_id.clone(), kind)
                    .expect("the local zone has an identifier"),
            )?;
        }
        if self.scenario() == DemoScenario::HistoryWeek {
            for adjustment in self.history_week_adjustments(&today) {
                corrections.save(&adjustment)?;
            }
        }
        Ok(())
    }

    /// The stored week has a long lock to review on Wednesday, a lunch lock on Tuesday, a sleep
    /// on Thursday and a lock still open today.
    fn seeded_transitions(
        &self,
        today: &LocalWorkday,
    ) -> Vec<(String, Timestamp, WorkdayTransitionKind)> {
        let session = WorkdayTransitionKind::SessionObserved;
        let power = WorkdayTransitionKind::PowerObserved;
        let at = |day: &LocalWorkday, hour, minute, second| {
            self.instant(day, time(hour, minute, second, 0))
        };
        match self.scenario() {
            DemoScenario::HistoryWeek => {
                let (tuesday, wednesday, thursday) = (
                    days_from(today, -3),
                    days_from(today, -2),
                    days_from(today, -1),
                );
                [
                    (
                        "lock",
                        at(&wednesday, 10, 0, 0),
                        session(SessionState::Locked),
                    ),
                    (
                        "unlock",
                        at(&wednesday, 12, 0, 0),
                        session(SessionState::Unlocked),
                    ),
                    (
                        "open-lock",
                        at(today, 14, 0, 0),
                        session(SessionState::Locked),
                    ),
                    (
                        "lunch-lock",
                        at(&tuesday, 12, 15, 0),
                        session(SessionState::Locked),
                    ),
                    (
                        "lunch-unlock",
                        at(&tuesday, 13, 0, 0),
                        session(SessionState::Unlocked),
                    ),
                    (
                        "suspend",
                        at(&thursday, 11, 0, 0),
                        power(PowerState::Suspended),
                    ),
                    (
                        "resume",
                        at(&thursday, 11, 40, 0),
                        power(PowerState::Active),
                    ),
                ]
                .into_iter()
                .map(|(key, at, kind)| (format!("demo-history-week-{key}"), at, kind))
                .collect()
            }
            DemoScenario::ProblemPeriods => [
                ("suspend", at(today, 10, 0, 0), power(PowerState::Suspended)),
                ("resume", at(today, 10, 30, 0), power(PowerState::Active)),
            ]
            .into_iter()
            .map(|(key, at, kind)| (format!("demo-problem-{key}"), at, kind))
            .collect(),
            DemoScenario::FragmentedDay => [
                ((0, 0, 0), PowerState::Suspended),
                ((8, 27, 8), PowerState::Active),
                ((10, 45, 0), PowerState::Suspended),
                ((10, 45, 20), PowerState::Active),
            ]
            .into_iter()
            .map(|((hour, minute, second), state)| {
                (
                    format!("demo-fragmented-{hour:02}{minute:02}{second:02}"),
                    at(today, hour, minute, second),
                    power(state),
                )
            })
            .collect(),
            _ => Vec::new(),
        }
    }

    /// Half an hour of manual credit on Monday and a meeting declared on Tuesday, recorded an
    /// hour before the clock.
    fn history_week_adjustments(&self, today: &LocalWorkday) -> [ManualAdjustment; 2] {
        let (monday, tuesday) = (days_from(today, -4), days_from(today, -3));
        let created_at = self.now() - SignedDuration::from_hours(1);
        let id = |value: &str| AdjustmentId::new(value).expect("the demo ids are not blank");
        let meeting = UtcInterval::new(
            self.instant(&tuesday, time(15, 15, 0, 0)),
            self.instant(&tuesday, time(16, 0, 0, 0)),
        )
        .expect("the meeting has a duration");
        [
            ManualAdjustment::duration_work(
                id("demo-history-week-manual"),
                monday,
                SignedDuration::from_mins(30),
                created_at,
            )
            .expect("Monday is a working day"),
            ManualAdjustment::interval_work(
                id("demo-history-week-meeting"),
                tuesday,
                meeting,
                created_at,
            )
            .expect("Tuesday is a working day"),
        ]
        .map(|adjustment| adjustment.with_reason(HISTORY_WEEK_REASON))
    }

    /// The summary the demo shows: the stored scenarios through the weekly recalculation, as the
    /// app refreshes, the others as one day. `weekly` recalculates through `accounting`.
    pub fn refresh(
        &self,
        accounting: &WorkdayAccounting<'_>,
        weekly: &WeeklyRecalculation<'_>,
        previous: Option<&WorkdayRefreshState>,
    ) -> Result<WorkdayRefreshState, WeeklyError> {
        if self.scenario().is_stored() {
            return weekly.refresh(previous);
        }
        Ok(WorkdayRefreshState {
            summary: Some(self.summary(accounting)?),
            source_failure: None,
        })
    }

    /// Today's summary without the ledger's weekly credit. The credit scenario credits its
    /// Monday with an hour over the target, so Tuesday shows that hour applied.
    pub fn summary(
        &self,
        accounting: &WorkdayAccounting<'_>,
    ) -> Result<DailySummary, AccountingError> {
        let context = accounting.capture_context();
        let today = context.calendar().workday(context.observed_at).date;
        let calculation = accounting.calculate(today, &context)?;
        let summary_input = &calculation.summary_input;
        if self.scenario() != DemoScenario::Credit {
            return Ok(summaries::build_nominal(summary_input)
                .expect("a summary of a domain calculation is consistent"));
        }

        let monday = today.yesterday().expect("the demo Tuesday has a Monday");
        let accounted = summary_input.accounting();
        let credit = calculate_week(
            &[
                DailyCreditInput::new(
                    monday,
                    context.settings.nominal_daily_target + SignedDuration::from_hours(1),
                ),
                DailyCreditInput {
                    date: today,
                    gross_credited_work: accounted.credited_work(),
                    work_awaiting_review: accounted.work_awaiting_review(),
                    is_finished: false,
                },
            ],
            &context.settings,
        )
        .expect("validated settings and a consecutive week from Monday")[1];
        Ok(summaries::build_with_credit(summary_input, credit)
            .expect("the credit result was calculated for this day"))
    }

    fn workday(&self) -> LocalWorkday {
        LocalWorkday::new(self.scenario().date(), self.zones.local().0)
    }

    fn instant(&self, workday: &LocalWorkday, local: Time) -> Timestamp {
        WorkdayCalendar::new(&self.zones)
            .instant(workday, local)
            .expect("the local zone resolves")
    }

    fn at(&self, hour: i8, minute: i8) -> Timestamp {
        self.instant(&self.workday(), time(hour, minute, 0, 0))
    }

    fn read_scenario(&self, bounds: UtcInterval) -> ActivityReadResult {
        let origin = origin(SYNTHETIC_ORIGIN);
        let segment = |start: Timestamp, end: Timestamp, state: ActivityState| {
            ActivitySegment::new(
                UtcInterval::new(start, end).expect("the demo segments have a duration"),
                state,
                origin.clone(),
            )
        };
        let at = |hour, minute| self.at(hour, minute);
        let (afk, active, unknown) = (
            ActivityState::Afk,
            ActivityState::Active,
            ActivityState::Unknown,
        );
        let start = match self.scenario() {
            DemoScenario::MorningWarning => at(9, 20),
            DemoScenario::MorningLate => at(9, 40),
            DemoScenario::Warning => at(10, 20),
            DemoScenario::Late => at(11, 0),
            _ => at(9, 0),
        };
        let segments = match self.scenario() {
            DemoScenario::Credit => {
                let worked = start + SignedDuration::from_mins(12);
                vec![
                    segment(at(0, 0), start, afk),
                    segment(start, worked, active),
                    segment(worked, at(23, 59), afk),
                ]
            }
            DemoScenario::ProblemPeriods => {
                let first_start = at(9, 40) + SignedDuration::from_secs(12);
                let first_end = at(9, 45) + SignedDuration::from_secs(42);
                vec![
                    segment(at(0, 0), at(9, 0), afk),
                    segment(at(9, 0), first_start, active),
                    segment(first_start, first_end, unknown),
                    segment(first_end, at(12, 0), active),
                    segment(at(12, 0), at(13, 0), afk),
                    segment(at(13, 0), at(14, 0), active),
                    segment(at(14, 0), at(14, 20), unknown),
                    segment(at(14, 20), at(23, 59), active),
                ]
            }
            DemoScenario::FragmentedDay => {
                let mut ends = vec![
                    (at(8, 27) + SignedDuration::from_secs(8), afk),
                    (at(9, 40), active),
                    (at(9, 44) + SignedDuration::from_secs(30), unknown),
                ];
                for index in 0..12 {
                    let fragment_end = at(10, 0) + SignedDuration::from_mins(index * 3);
                    ends.push((fragment_end, active));
                    ends.push((
                        fragment_end + SignedDuration::from_millis(100 + index),
                        unknown,
                    ));
                }
                ends.extend([(at(12, 0), active), (at(13, 0), afk), (at(23, 59), active)]);
                let mut cursor = at(0, 0);
                ends.into_iter()
                    .map(|(end, state)| {
                        let fragment = segment(cursor, end, state);
                        cursor = end;
                        fragment
                    })
                    .collect()
            }
            _ => vec![
                segment(at(0, 0), start, afk),
                segment(start, at(12, 0), active),
                segment(at(12, 0), at(13, 0), afk),
                segment(at(13, 0), at(23, 59), active),
            ],
        };
        ActivityReadResult::Snapshot {
            coverage: ActivityCoverage::new(bounds, segments),
            health: SourceHealth::healthy(SOURCE_ID, bounds.end()),
        }
    }

    /// Any day the week view asks for: 9:00 to 17:30 on Monday, 7 hours from 8:15 on Tuesday,
    /// 7 h 45 min with a quarter hour unknown at 14:00 on Wednesday, 6 h 30 min on Thursday and
    /// 7 hours on the other days, active from 9:00.
    fn read_history_week(&self, bounds: UtcInterval) -> ActivityReadResult {
        let (zone_id, zone) = self.zones.local();
        let date = bounds.start().to_zoned(zone).date();
        let workday = LocalWorkday::new(date, zone_id);
        let local_at = |hour| self.instant(&workday, time(hour, 0, 0, 0));
        let worked = match date.weekday() {
            Weekday::Monday => SignedDuration::from_mins(8 * 60 + 30),
            Weekday::Wednesday => SignedDuration::from_mins(7 * 60 + 45),
            Weekday::Thursday => SignedDuration::from_mins(6 * 60 + 30),
            _ => SignedDuration::from_hours(7),
        };
        // Tuesday starts and ends 45 minutes early so its declared meeting ends by 16:00, the
        // credit scenario's clock on that same day: the seeded data stays in the ledger when the
        // picker switches to the credit scenario, and a later interval is past its horizon.
        let active_start = if date.weekday() == Weekday::Tuesday {
            local_at(8) + SignedDuration::from_mins(15)
        } else {
            local_at(9)
        };
        let active_end = active_start + worked;
        let credited_start = active_start.clamp(bounds.start(), bounds.end());
        let credited_end = active_end.min(bounds.end()).max(credited_start);

        let origin = origin(HISTORY_WEEK_ORIGIN);
        let mut segments = Vec::new();
        let mut push = |start: Timestamp, end: Timestamp, state| {
            if let Ok(interval) = UtcInterval::new(start, end) {
                segments.push(ActivitySegment::new(interval, state, origin.clone()));
            }
        };
        push(bounds.start(), credited_start, ActivityState::Afk);
        let unknown_start = local_at(14);
        let unknown_end = unknown_start + SignedDuration::from_mins(15);
        if date.weekday() == Weekday::Wednesday
            && credited_start < unknown_start
            && unknown_end < credited_end
        {
            push(credited_start, unknown_start, ActivityState::Active);
            push(unknown_start, unknown_end, ActivityState::Unknown);
            push(unknown_end, credited_end, ActivityState::Active);
        } else {
            push(credited_start, credited_end, ActivityState::Active);
        }
        push(credited_end, bounds.end(), ActivityState::Afk);
        ActivityReadResult::Snapshot {
            coverage: ActivityCoverage::new(bounds, segments),
            health: SourceHealth::healthy(SOURCE_ID, bounds.end()),
        }
    }
}

impl<Z: TimeZoneProvider> Clock for DemoEnvironment<Z> {
    fn now(&self) -> Timestamp {
        self.instant(&self.workday(), self.scenario().local_now())
    }
}

impl<Z: TimeZoneProvider> ActivitySource for DemoEnvironment<Z> {
    fn read(&self, interval: UtcInterval) -> Result<ActivityReadResult, ActivityError> {
        Ok(match self.scenario() {
            scenario @ (DemoScenario::Unavailable | DemoScenario::ActivityWatchUnavailable) => {
                ActivityReadResult::Unavailable {
                    health: SourceHealth::degraded(
                        if scenario == DemoScenario::ActivityWatchUnavailable {
                            ACTIVITY_WATCH_ID
                        } else {
                            SOURCE_ID
                        },
                        SourceFreshness::Unavailable,
                        SourceQuality::Incomplete,
                        OFFLINE_REASON,
                        None,
                    )
                    .expect("an unavailable source is degraded"),
                }
            }
            DemoScenario::HistoryWeek => self.read_history_week(interval),
            _ => self.read_scenario(interval),
        })
    }
}

fn days_from(day: &LocalWorkday, offset: i64) -> LocalWorkday {
    LocalWorkday::new(
        day.date
            .checked_add(offset.days())
            .expect("the demo week is far from the calendar's end"),
        day.time_zone_id.clone(),
    )
}

fn origin(source_id: &str) -> AutomaticOrigin {
    AutomaticOrigin::new(source_id).expect("the demo origins are not blank")
}

#[cfg(test)]
mod tests;
