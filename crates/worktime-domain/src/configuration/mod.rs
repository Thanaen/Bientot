//! User settings and their validation. Reading and writing `settings.json` belongs to the
//! persistence layer.

mod proposal;
mod validator;

use jiff::civil::{Date, Time, Weekday};
use jiff::{SignedDuration, Timestamp};

use crate::sources::{ActivitySourceKind, IdleStateTracker};

pub use proposal::{AppointmentProposal, ObservedDay, Occurrence};
pub use validator::SettingsError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkTimeSettings {
    pub schema_version: i32,
    pub nominal_daily_target: SignedDuration,
    pub minimum_effective_target: SignedDuration,
    pub lunch_start: Time,
    pub lunch_end: Time,
    pub preferred_latest_departure: Time,
    pub orange_warning_margin: SignedDuration,
    pub long_lock_review_threshold: SignedDuration,
    pub weekly_credit_enabled: bool,
    /// Lets finished days under their target leave a weekly debt; effective only with weekly
    /// credit.
    pub weekly_debt_enabled: bool,
    /// The largest weekly debt kept; any further shortfall is dropped. `None` means no limit.
    pub maximum_weekly_debt: Option<SignedDuration>,
    pub start_with_windows: bool,
    pub notifications: NotificationSettings,
    /// Mirrors the latest activity source selection. `None` in a settings file written before
    /// the built-in source existed, whose onboarding was completed with ActivityWatch.
    pub activity_source: Option<ActivitySourceKind>,
    pub built_in_activity: BuiltInActivitySettings,
    pub activity_watch: ActivityWatchSettings,
    pub updates: UpdateSettings,
    /// Sends the anonymous daily usage signal (`crate::usage_ping`). Absent from the .NET app,
    /// which skips it on read and drops it on write.
    pub usage_ping: bool,
    /// Absent from the .NET app, which skips it on read and drops it on write.
    pub recurring_appointments: Vec<RecurringAppointment>,
    /// Absent from the .NET app, which skips it on read and drops it on write.
    pub appointment_proposals: AppointmentProposalSettings,
    pub logging: LoggingSettings,
    pub language_override: Option<String>,
}

impl WorkTimeSettings {
    pub const CURRENT_SCHEMA_VERSION: i32 = 1;
}

impl Default for WorkTimeSettings {
    fn default() -> Self {
        Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            nominal_daily_target: SignedDuration::from_hours(7),
            minimum_effective_target: SignedDuration::from_hours(6),
            lunch_start: Time::constant(12, 0, 0, 0),
            lunch_end: Time::constant(14, 0, 0, 0),
            preferred_latest_departure: Time::constant(18, 30, 0, 0),
            orange_warning_margin: SignedDuration::from_mins(15),
            long_lock_review_threshold: SignedDuration::from_mins(90),
            weekly_credit_enabled: true,
            weekly_debt_enabled: false,
            maximum_weekly_debt: None,
            start_with_windows: true,
            notifications: NotificationSettings::default(),
            activity_source: None,
            built_in_activity: BuiltInActivitySettings::default(),
            activity_watch: ActivityWatchSettings::default(),
            updates: UpdateSettings::default(),
            usage_ping: true,
            recurring_appointments: Vec::new(),
            appointment_proposals: AppointmentProposalSettings::default(),
            logging: LoggingSettings::default(),
            language_override: None,
        }
    }
}

/// A meeting or another appointment that comes back each week at the same hours, during which
/// the user may work away from the PC (issue #321).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurringAppointment {
    /// Kept across edits; the reviews of the appointment are named after it.
    pub id: String,
    /// Empty for the name the app derives from the weekday.
    pub name: String,
    pub weekday: Weekday,
    pub start: Time,
    pub end: Time,
    pub enabled: bool,
    /// The first day it counts on, so that adding it leaves the days before as they were.
    /// `None` counts it on every day.
    pub since: Option<Date>,
    /// One week in two, the weeks of this day and every second one before and after it; it
    /// falls on `weekday`. `None` is every week.
    pub alternate_weeks_from: Option<Date>,
    /// « Compter automatiquement »: an absence during it is counted as work once the user is
    /// back, without asking.
    pub automatic: bool,
}

impl RecurringAppointment {
    /// The most appointments the settings keep.
    pub const MAXIMUM_COUNT: usize = 20;
    pub const MAXIMUM_NAME_LENGTH: usize = 80;
    pub const MAXIMUM_ID_LENGTH: usize = 64;

    /// Whether the appointment, switched on, counts on `date`.
    pub fn falls_on(&self, date: Date) -> bool {
        self.enabled
            && self.weekday == date.weekday()
            && self.since.is_none_or(|since| since <= date)
            && self
                .alternate_weeks_from
                .is_none_or(|from| Self::same_alternate_weeks(date, from))
    }

    /// Whether two days on one weekday fall in the same weeks of an appointment one week in
    /// two: a whole number of fortnights apart.
    pub fn same_alternate_weeks(one: Date, other: Date) -> bool {
        (one - other).get_days().rem_euclid(14) == 0
    }

    /// Whether two `alternate_weeks_from` repeat on the same weeks.
    pub fn same_repeat(one: Option<Date>, other: Option<Date>) -> bool {
        match (one, other) {
            (None, None) => true,
            (Some(one), Some(other)) => Self::same_alternate_weeks(one, other),
            _ => false,
        }
    }
}

/// The appointments WorkTime proposes after repeated corrections, see [`AppointmentProposal`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppointmentProposalSettings {
    /// « Proposer des rendez-vous d’après vos corrections ».
    pub enabled: bool,
    /// The proposals the user turned down, newest last.
    pub dismissed: Vec<DismissedProposal>,
}

impl Default for AppointmentProposalSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            dismissed: Vec::new(),
        }
    }
}

/// A proposal the user turned down: WorkTime proposes nothing on `weekday` over these hours
/// again before `until`, or ever for « Ne plus proposer pour le jeudi ».
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DismissedProposal {
    pub weekday: Weekday,
    pub start: Time,
    pub end: Time,
    pub until: Option<Date>,
}

impl DismissedProposal {
    /// The most refusals the settings keep; the oldest go first.
    pub const MAXIMUM_COUNT: usize = 20;
    /// « Annuler » waits this long before proposing the same again.
    pub const POSTPONED_DAYS: i32 = 14;

    /// Whether it still keeps `proposal` back on `today`: « Ne plus proposer pour les jeudis »
    /// any proposal on its weekday, a postponement those over the same hours until it ends.
    pub fn holds(&self, proposal: &AppointmentProposal, today: Date) -> bool {
        match self.until {
            None => true,
            Some(until) => today < until && self.start < proposal.end && proposal.start < self.end,
        }
    }
}

impl AppointmentProposalSettings {
    /// Records `dismissal` as of `today` in place of the refusals of the same hours on its
    /// weekday and of the postponements over, dropping the oldest beyond
    /// [`DismissedProposal::MAXIMUM_COUNT`], postponements first.
    pub fn dismiss(&mut self, dismissal: DismissedProposal, today: Date) {
        self.dismissed.retain(|kept| {
            kept.until.is_none_or(|until| today < until)
                && (kept.weekday != dismissal.weekday
                    || kept.end <= dismissal.start
                    || dismissal.end <= kept.start)
        });
        self.dismissed.push(dismissal);
        while self.dismissed.len() > DismissedProposal::MAXIMUM_COUNT {
            let oldest = self
                .dismissed
                .iter()
                .position(|kept| kept.until.is_some())
                .unwrap_or(0);
            self.dismissed.remove(oldest);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotificationSettings {
    pub completion_approach: NotificationRule,
    pub latest_departure_approach: NotificationRule,
    pub day_complete: NotificationRule,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            completion_approach: NotificationRule::new(true, SignedDuration::from_mins(15)),
            latest_departure_approach: NotificationRule::new(true, SignedDuration::from_mins(15)),
            day_complete: NotificationRule::new(false, SignedDuration::ZERO),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotificationRule {
    pub enabled: bool,
    pub lead_time: SignedDuration,
}

impl NotificationRule {
    pub fn new(enabled: bool, lead_time: SignedDuration) -> Self {
        Self { enabled, lead_time }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltInActivitySettings {
    pub idle_threshold: SignedDuration,
}

impl BuiltInActivitySettings {
    pub const DEFAULT_IDLE_THRESHOLD: SignedDuration = SignedDuration::from_secs(180);
    pub const MINIMUM_IDLE_THRESHOLD: SignedDuration = SignedDuration::from_secs(30);
    pub const MAXIMUM_IDLE_THRESHOLD: SignedDuration = SignedDuration::from_mins(30);
    const MAXIMUM_JITTER: SignedDuration = SignedDuration::from_secs(5);

    /// How long the uncovered live tail of a built-in read can still settle: the threshold, one
    /// polling interval, and jitter.
    pub fn pending_horizon(&self) -> SignedDuration {
        self.idle_threshold + IdleStateTracker::DEFAULT_POLL_INTERVAL + Self::MAXIMUM_JITTER
    }
}

impl Default for BuiltInActivitySettings {
    fn default() -> Self {
        Self {
            idle_threshold: Self::DEFAULT_IDLE_THRESHOLD,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityWatchSettings {
    pub base_url: String,
    pub selected_bucket_id: Option<String>,
    pub api_key_reference: Option<String>,
    pub request_timeout: SignedDuration,
    pub settlement: ActivityWatchSettlementContract,
}

impl Default for ActivityWatchSettings {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:5600".to_owned(),
            selected_bucket_id: None,
            api_key_reference: None,
            request_timeout: SignedDuration::from_secs(5),
            settlement: ActivityWatchSettlementContract::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActivityWatchSettlementContract {
    pub afk_timeout: SignedDuration,
    pub poll_interval: SignedDuration,
    pub maximum_jitter: SignedDuration,
}

impl ActivityWatchSettlementContract {
    pub fn horizon(&self) -> SignedDuration {
        self.afk_timeout + self.poll_interval + self.maximum_jitter
    }
}

impl Default for ActivityWatchSettlementContract {
    fn default() -> Self {
        Self {
            afk_timeout: SignedDuration::from_secs(180),
            poll_interval: SignedDuration::from_secs(5),
            maximum_jitter: SignedDuration::from_secs(5),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpdateSettings {
    pub check_for_updates: bool,
    pub last_checked_at: Option<Timestamp>,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            check_for_updates: true,
            last_checked_at: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoggingSettings {
    pub maximum_file_count: i32,
    pub maximum_file_bytes: i64,
}

impl Default for LoggingSettings {
    fn default() -> Self {
        Self {
            maximum_file_count: 5,
            maximum_file_bytes: 1_048_576,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_pending_horizon_adds_one_polling_interval_and_five_seconds_of_jitter() {
        let settings = BuiltInActivitySettings {
            idle_threshold: SignedDuration::from_secs(180),
        };

        assert_eq!(settings.pending_horizon(), SignedDuration::from_secs(190));
    }
}
