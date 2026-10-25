//! Reminders shown as the workday approaches its end, ported from
//! `Services/NotificationCoordinator.cs` and `Services/TwoScenarioProjector.cs`, with the choice
//! of message from `Presentation/Notifications/NotificationTextProvider.cs`.
//!
//! The coordinator buffers the approach reminders of one UTC minute and sends them together once
//! a later minute is observed, so that samples split across a minute make one toast. Each trigger
//! is shown at most once per workday: the receipt ledger reserves it before the toast is shown,
//! across restarts, and a toast that fails to show is not retried. The wording stays with the
//! caller, which turns a [`Notification`] into text and a toast.

use std::collections::{BTreeMap, BTreeSet};

use bientot_domain::accounting::SegmentKind;
use bientot_domain::configuration::Settings;
use bientot_domain::forecasting::{DepartureProjection, ForecastAvailability, project_departure};
use bientot_domain::intervals::UtcInterval;
use bientot_domain::notifications::{NotificationPolicyInput, NotificationTrigger, eligible};
use bientot_domain::summaries::DailySummary;
use bientot_domain::time::{CalendarError, LocalWorkday, TimeZoneProvider, WorkdayCalendar};
use bientot_store::notifications::{
    NotificationClaim, NotificationReceiptError, NotificationReceiptRepository,
};
use jiff::{SignedDuration, Timestamp};
use sha2::{Digest, Sha256};

use crate::refresh::Publication;
use crate::weekly::WorkdayRefreshState;

/// The resource key of every toast's title.
pub const TITLE_RESOURCE_KEY: &str = "NotificationTitle";
/// The resource key of the sentence appended to the body of an estimated reminder.
pub const ESTIMATE_QUALIFIER_RESOURCE_KEY: &str = "NotificationEstimateQualifier";

/// Shows a toast: in the app, `bientot-windows::notifications::Toasts` with the text the
/// caller writes.
pub trait NotificationSink {
    fn show(
        &self,
        notification: &Notification,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// One toast: the triggers it was reserved for and what its text reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub delivery_id: String,
    pub triggers: BTreeSet<NotificationTrigger>,
    pub remaining_work: SignedDuration,
    pub projected_departure: Option<Timestamp>,
    /// The zone the projected departure is shown in.
    pub time_zone_id: String,
    /// The summary counted only recorded work, so the text says so.
    pub is_estimated: bool,
}

impl Notification {
    pub fn body(&self) -> NotificationBody {
        let completion = self
            .triggers
            .contains(&NotificationTrigger::CompletionApproach);
        let departure = self
            .triggers
            .contains(&NotificationTrigger::LatestDepartureApproach);
        if self.triggers.contains(&NotificationTrigger::DayComplete)
            || (completion && self.remaining_minutes() == 0)
        {
            NotificationBody::Complete
        } else if completion && departure {
            NotificationBody::Combined
        } else if completion {
            NotificationBody::CompletionApproach
        } else {
            NotificationBody::LatestDepartureApproach
        }
    }

    /// The remaining work in whole minutes, rounded up.
    pub fn remaining_minutes(&self) -> i64 {
        const MINUTE: i128 = 60_000_000_000;
        let nanoseconds = self.remaining_work.as_nanos().max(0);
        i64::try_from((nanoseconds + MINUTE - 1) / MINUTE)
            .expect("a day's remaining work fits in minutes")
    }
}

/// Which sentence a toast's body is. Each formats the remaining minutes and the projected
/// departure; [`NotificationBody::Complete`] uses neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationBody {
    CompletionApproach,
    LatestDepartureApproach,
    Combined,
    Complete,
}

impl NotificationBody {
    pub fn resource_key(self) -> &'static str {
        match self {
            Self::CompletionApproach => "NotificationCompletionBody",
            Self::LatestDepartureApproach => "NotificationDepartureBody",
            Self::Combined => "NotificationCombinedBody",
            Self::Complete => "NotificationCompleteBody",
        }
    }
}

#[derive(Debug)]
pub enum NotificationError {
    Calendar(CalendarError),
    Receipts(NotificationReceiptError),
    /// The triggers stay reserved, so the toast is not shown again.
    Show(Box<dyn std::error::Error + Send + Sync>),
}

impl std::fmt::Display for NotificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Calendar(error) => error.fmt(f),
            Self::Receipts(error) => error.fmt(f),
            Self::Show(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for NotificationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Calendar(error) => Some(error),
            Self::Receipts(error) => Some(error),
            Self::Show(error) => Some(error.as_ref()),
        }
    }
}

impl From<CalendarError> for NotificationError {
    fn from(error: CalendarError) -> Self {
        Self::Calendar(error)
    }
}

impl From<NotificationReceiptError> for NotificationError {
    fn from(error: NotificationReceiptError) -> Self {
        Self::Receipts(error)
    }
}

/// The approach reminders eligible during one UTC minute, the first claim of each kept.
struct PendingMinute {
    minute: i128,
    claims: BTreeMap<NotificationTrigger, NotificationClaim>,
}

/// Decides which published summaries notify. Publications are expected one at a time, in the
/// order the refresh coordinator numbered them.
pub struct NotificationCoordinator<'a> {
    receipts: NotificationReceiptRepository<'a>,
    sink: &'a dyn NotificationSink,
    zones: &'a dyn TimeZoneProvider,
    settings: Box<dyn Fn() -> Settings + 'a>,
    pending: Option<PendingMinute>,
    workday: Option<LocalWorkday>,
    latest_revision: u64,
    latest_instant: Option<Timestamp>,
}

impl<'a> NotificationCoordinator<'a> {
    pub fn new(
        receipts: NotificationReceiptRepository<'a>,
        sink: &'a dyn NotificationSink,
        zones: &'a dyn TimeZoneProvider,
        settings: Box<dyn Fn() -> Settings + 'a>,
    ) -> Self {
        Self {
            receipts,
            sink,
            zones,
            settings,
            pending: None,
            workday: None,
            latest_revision: 0,
            latest_instant: None,
        }
    }

    /// Observes a published refresh. Only a summary the source fully answered for notifies; a
    /// refresh that kept the last summary beside a source failure is ignored. An older revision
    /// is ignored, and a clock that went back or a new workday drops the buffered reminders.
    pub fn observe(
        &mut self,
        publication: &Publication<WorkdayRefreshState>,
    ) -> Result<(), NotificationError> {
        let WorkdayRefreshState {
            summary: Some(summary),
            source_failure: None,
        } = &publication.summary
        else {
            return Ok(());
        };
        if publication.revision <= self.latest_revision {
            return Ok(());
        }
        let computed_at = summary.computed_at();
        if self.latest_instant.is_some_and(|last| computed_at < last) {
            self.pending = None;
        }
        self.latest_revision = publication.revision;
        self.latest_instant = Some(computed_at);
        if self
            .workday
            .as_ref()
            .is_some_and(|workday| workday != summary.workday())
        {
            self.pending = None;
        }
        self.workday = Some(summary.workday().clone());

        let minute = utc_minute(computed_at);
        if let Some(buffered) = self.pending.take_if(|pending| minute > pending.minute) {
            self.send(buffered.claims.into_values().collect(), summary)?;
        }

        let settings = (self.settings)();
        let calendar = WorkdayCalendar::new(self.zones);
        let mut approaches = eligible_triggers(summary, &settings, &calendar)?;
        if approaches.remove(&NotificationTrigger::DayComplete) {
            self.send(
                vec![claim(summary, NotificationTrigger::DayComplete)],
                summary,
            )?;
        }
        if approaches.is_empty() {
            return Ok(());
        }
        let mut pending = self
            .pending
            .take()
            .filter(|pending| pending.minute == minute)
            .unwrap_or(PendingMinute {
                minute,
                claims: BTreeMap::new(),
            });
        for trigger in approaches {
            pending
                .claims
                .entry(trigger)
                .or_insert_with(|| claim(summary, trigger));
        }
        self.pending = Some(pending);
        Ok(())
    }

    /// Shows the claims still valid for `current` that no earlier toast reserved.
    fn send(
        &self,
        claims: Vec<NotificationClaim>,
        current: &DailySummary,
    ) -> Result<(), NotificationError> {
        if claims.is_empty()
            || claims
                .iter()
                .any(|claim| claim.workday != *current.workday())
        {
            return Ok(());
        }
        let settings = (self.settings)();
        let calendar = WorkdayCalendar::new(self.zones);
        let mut valid = Vec::with_capacity(claims.len());
        for claim in claims {
            if claim_is_valid(claim.trigger, current, &settings, &calendar)? {
                valid.push(claim);
            }
        }
        if valid.is_empty() {
            return Ok(());
        }

        let delivery_id = delivery_id(current, &valid);
        let reserved = self
            .receipts
            .try_reserve(&valid, &delivery_id, current.computed_at())?;
        if reserved.is_empty() {
            return Ok(());
        }
        self.sink
            .show(&Notification {
                delivery_id,
                triggers: reserved,
                remaining_work: current.remaining_work(),
                projected_departure: current
                    .departure_projection()
                    .map(|projection| projection.projected_departure),
                time_zone_id: current.workday().time_zone_id.clone(),
                is_estimated: current.assessment().forecast() == ForecastAvailability::Estimated,
            })
            .map_err(NotificationError::Show)
    }
}

fn claim(summary: &DailySummary, trigger: NotificationTrigger) -> NotificationClaim {
    NotificationClaim {
        workday: summary.workday().clone(),
        trigger,
        eligible_at: summary.computed_at(),
    }
}

/// The index of the UTC minute holding `instant`.
fn utc_minute(instant: Timestamp) -> i128 {
    instant.as_nanosecond().div_euclid(60_000_000_000)
}

/// 32 hex digits, as the .NET GUID was, derived from the claims so that a toast keeps its id
/// however often the same summary is retried.
fn delivery_id(current: &DailySummary, claims: &[NotificationClaim]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(current.workday().date.to_string());
    hasher.update([0]);
    hasher.update(&current.workday().time_zone_id);
    hasher.update([0]);
    hasher.update(current.computed_at().as_nanosecond().to_le_bytes());
    for claim in claims {
        hasher.update([claim.trigger as u8]);
    }
    hasher.finalize()[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// How the day ends if the time the accounting does not know is not work, and if all of it is.
struct Scenarios {
    known_only: Scenario,
    possible_credit: Scenario,
}

struct Scenario {
    remaining_work: SignedDuration,
    projection: Option<DepartureProjection>,
}

/// A confirmed summary notifies as the known work allows; an estimated one only for the triggers
/// both scenarios allow, so that unknown time cannot make a reminder premature.
fn eligible_triggers(
    summary: &DailySummary,
    settings: &Settings,
    calendar: &WorkdayCalendar<&dyn TimeZoneProvider>,
) -> Result<BTreeSet<NotificationTrigger>, CalendarError> {
    let forecast = summary.assessment().forecast();
    if forecast == ForecastAvailability::Unavailable {
        return Ok(BTreeSet::new());
    }
    let scenarios = scenarios(summary, settings, calendar)?;
    let known_only = eligible(&policy_input(
        summary,
        settings,
        calendar,
        &scenarios.known_only,
    )?);
    if forecast == ForecastAvailability::Confirmed {
        return Ok(known_only);
    }
    let possible_credit = eligible(&policy_input(
        summary,
        settings,
        calendar,
        &scenarios.possible_credit,
    )?);
    Ok(known_only.intersection(&possible_credit).copied().collect())
}

/// Whether a buffered claim still holds for the latest summary. A confirmed completion reminder
/// holds once the day completed, which the policy alone would no longer allow.
fn claim_is_valid(
    trigger: NotificationTrigger,
    summary: &DailySummary,
    settings: &Settings,
    calendar: &WorkdayCalendar<&dyn TimeZoneProvider>,
) -> Result<bool, CalendarError> {
    let forecast = summary.assessment().forecast();
    if forecast == ForecastAvailability::Unavailable {
        return Ok(false);
    }
    let scenarios = scenarios(summary, settings, calendar)?;
    let allows = |scenario: &Scenario| -> Result<bool, CalendarError> {
        Ok(eligible(&policy_input(summary, settings, calendar, scenario)?).contains(&trigger))
    };
    if forecast == ForecastAvailability::Estimated {
        return Ok(allows(&scenarios.known_only)? && allows(&scenarios.possible_credit)?);
    }
    if trigger == NotificationTrigger::CompletionApproach {
        let rule = settings.notifications.completion_approach;
        return Ok(rule.enabled && scenarios.known_only.remaining_work <= rule.lead_time);
    }
    allows(&scenarios.known_only)
}

fn scenarios(
    summary: &DailySummary,
    settings: &Settings,
    calendar: &WorkdayCalendar<&dyn TimeZoneProvider>,
) -> Result<Scenarios, CalendarError> {
    let lunch = calendar.interval(summary.workday(), settings.lunch_start, settings.lunch_end)?;
    let unknown_work = summary
        .accounting()
        .timeline()
        .segments()
        .iter()
        .filter(|segment| *segment.kind() == SegmentKind::Unknown)
        .fold(SignedDuration::ZERO, |total, segment| {
            total + segment.interval().duration()
        });
    Ok(Scenarios {
        known_only: project(summary, lunch, summary.credited_work()),
        possible_credit: project(summary, lunch, summary.credited_work() + unknown_work),
    })
}

fn project(summary: &DailySummary, lunch: UtcInterval, credited_work: SignedDuration) -> Scenario {
    let target = summary.effective_target();
    Scenario {
        remaining_work: (target - credited_work).max(SignedDuration::ZERO),
        projection: project_departure(summary.computed_at(), credited_work, target, lunch)
            .expect("accounted work is never negative and a summary's target is positive"),
    }
}

fn policy_input(
    summary: &DailySummary,
    settings: &Settings,
    calendar: &WorkdayCalendar<&dyn TimeZoneProvider>,
    scenario: &Scenario,
) -> Result<NotificationPolicyInput, CalendarError> {
    Ok(NotificationPolicyInput {
        workday: summary.workday().clone(),
        observed_at: summary.computed_at(),
        remaining_work: scenario.remaining_work,
        is_complete: scenario.projection.is_none(),
        is_healthy: true,
        preferred_latest_departure: calendar
            .instant(summary.workday(), settings.preferred_latest_departure)?,
        settings: settings.notifications,
    })
}

#[cfg(test)]
mod tests;
