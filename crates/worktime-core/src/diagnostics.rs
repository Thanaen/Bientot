//! The diagnostic snapshot and its ZIP export, ported from `Diagnostics/DiagnosticSnapshot.cs`,
//! `Services/DiagnosticSnapshotFactory.cs` and `Diagnostics/DiagnosticExportService.cs`, and the
//! local log the export reads (see [`LocalDiagnosticLogger`]).
//!
//! The snapshot keeps only settings that tune the rules and the aggregates of the current
//! summary: no source address, bucket, key reference, language or path. The export adds a
//! manifest and a projection of the local log files that keeps each record's minute, level,
//! known event name and a few fixed categories, and drops everything else. Every value the
//! export writes is a number, a boolean or a name from a fixed list, so the JSON is written by
//! hand in the property order and spelling of the .NET serializer.

mod export;
mod json;
mod logger;
mod zip;

pub use export::{
    DiagnosticExport, DiagnosticExportError, DiagnosticExportResult, FinalPaths, NoFinalPathCheck,
};
pub use logger::{
    DiagnosticLogError, DiagnosticLogLevel, DiagnosticLogOptions, DiagnosticValue,
    LocalDiagnosticLogger,
};

use jiff::civil::{Date, Time};
use jiff::{SignedDuration, Timestamp};
use worktime_domain::configuration::{NotificationRule, WorkTimeSettings};
use worktime_domain::forecasting::{AssessmentReason, ForecastAvailability};
use worktime_domain::sources::{SourceFreshness, SourceQuality};
use worktime_domain::summaries::IndicatorState;
use worktime_store::codec::to_ticks;

use crate::weekly::WorkdayRefreshState;
use json::JsonObject;

/// A summary computed longer ago than this is reported as stale.
const MAXIMUM_CURRENT_SUMMARY_AGE: SignedDuration = SignedDuration::from_mins(2);

/// Declares an enum whose values are written as their own names, as .NET writes enums.
macro_rules! named_enum {
    ($(#[$meta:meta])* $vis:vis enum $name:ident { $($variant:ident),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        $vis enum $name {
            $($variant),+
        }

        impl $name {
            /// The name written to the export.
            $vis const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => stringify!($variant)),+
                }
            }
        }
    };
}
pub(crate) use named_enum;

named_enum! {
    pub enum DiagnosticInitializationState { Unknown, Starting, Ready, Failed }
}

named_enum! {
    pub enum DiagnosticSummaryAvailability { Unknown, Current, Stale, Unavailable }
}

named_enum! {
    pub enum DiagnosticArchitecture {
        Unknown, X86, X64, Arm, Arm64, Wasm, S390x, LoongArch64, Armv6, Ppc64le,
    }
}

named_enum! {
    pub enum DiagnosticIndicator { Unknown, Green, Orange, Red, Blue }
}

named_enum! {
    pub enum DiagnosticForecastAvailability { Unknown, Confirmed, Estimated, Unavailable }
}

named_enum! {
    pub enum DiagnosticAssessmentReason {
        Unknown,
        PendingSettlement,
        InteriorCompatibilitySeam,
        HistoricalMissingCoverage,
        CurrentMissingCoverage,
        SourceUnavailable,
        SourceStale,
        SourceProtocolFailure,
        ProvisionalLockReview,
    }
}

impl DiagnosticAssessmentReason {
    /// How many values the enum has; a summary never lists more reasons than this.
    pub const COUNT: usize = 9;
}

named_enum! {
    pub enum DiagnosticSourceFreshness { Unknown, Fresh, Stale, Unavailable }
}

named_enum! {
    pub enum DiagnosticSourceQuality {
        Unknown, Complete, Incomplete, Ambiguous, Unauthorized, Incompatible, Malformed,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticSnapshot {
    pub initialization: DiagnosticInitializationState,
    pub settings: Option<DiagnosticSettingsSnapshot>,
    pub runtime: DiagnosticRuntimeSnapshot,
    pub summary: Option<DiagnosticSummarySnapshot>,
}

/// Durations are in .NET ticks of 100 ns, times of day in minutes after midnight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagnosticSettingsSnapshot {
    pub nominal_daily_target_ticks: i64,
    pub minimum_effective_target_ticks: i64,
    pub lunch_start_minute_of_day: i32,
    pub lunch_end_minute_of_day: i32,
    pub preferred_latest_departure_minute_of_day: i32,
    pub orange_warning_margin_ticks: i64,
    pub long_lock_review_threshold_ticks: i64,
    pub weekly_credit_enabled: bool,
    pub start_with_windows: bool,
    pub notifications: DiagnosticNotificationSettingsSnapshot,
    pub activity_watch: DiagnosticActivityWatchTimingSnapshot,
    pub logging: DiagnosticLoggingSettingsSnapshot,
    pub check_for_updates: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagnosticNotificationSettingsSnapshot {
    pub completion_approach: DiagnosticNotificationRuleSnapshot,
    pub latest_departure_approach: DiagnosticNotificationRuleSnapshot,
    pub day_complete: DiagnosticNotificationRuleSnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagnosticNotificationRuleSnapshot {
    pub enabled: bool,
    pub lead_time_ticks: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagnosticActivityWatchTimingSnapshot {
    pub request_timeout_ticks: i64,
    pub afk_timeout_ticks: i64,
    pub poll_interval_ticks: i64,
    pub maximum_jitter_ticks: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagnosticLoggingSettingsSnapshot {
    pub maximum_file_count: i32,
    pub maximum_file_bytes: i64,
}

/// Supplied by the host, which knows its own version, the system's and the architectures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagnosticRuntimeSnapshot {
    pub application_version: DiagnosticNumericVersion,
    pub operating_system_version: DiagnosticNumericVersion,
    /// Written as `dotNetVersion`, the name the .NET app gave it.
    pub dot_net_version: DiagnosticNumericVersion,
    pub process_architecture: DiagnosticArchitecture,
    pub operating_system_architecture: DiagnosticArchitecture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagnosticNumericVersion {
    pub major: i32,
    pub minor: i32,
    pub build: i32,
    pub revision: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticSummarySnapshot {
    pub availability: DiagnosticSummaryAvailability,
    pub credited_work_ticks: i64,
    pub nominal_target_ticks: i64,
    pub effective_target_ticks: i64,
    pub remaining_work_ticks: i64,
    pub credited_overtime_ticks: i64,
    pub applied_carry_credit_ticks: i64,
    pub indicator: DiagnosticIndicator,
    pub forecast: DiagnosticForecastAvailability,
    pub assessment_reasons: Vec<DiagnosticAssessmentReason>,
    pub source_freshness: DiagnosticSourceFreshness,
    pub source_quality: DiagnosticSourceQuality,
}

/// The snapshot of the running app: `settings` and `refresh_state` are `None` until they were
/// loaded, and the summary is current only when it is today's, fresh, recent and not kept
/// beside a source failure.
pub fn create_snapshot(
    initialization: DiagnosticInitializationState,
    settings: Option<&WorkTimeSettings>,
    refresh_state: Option<&WorkdayRefreshState>,
    current_workday: Option<Date>,
    now: Timestamp,
    runtime: DiagnosticRuntimeSnapshot,
) -> DiagnosticSnapshot {
    DiagnosticSnapshot {
        initialization,
        settings: settings.map(settings_snapshot),
        runtime,
        summary: refresh_state.and_then(|state| summary_snapshot(state, current_workday, now)),
    }
}

fn settings_snapshot(settings: &WorkTimeSettings) -> DiagnosticSettingsSnapshot {
    let rule = |rule: NotificationRule| DiagnosticNotificationRuleSnapshot {
        enabled: rule.enabled,
        lead_time_ticks: ticks(rule.lead_time),
    };
    let activity_watch = &settings.activity_watch;
    DiagnosticSettingsSnapshot {
        nominal_daily_target_ticks: ticks(settings.nominal_daily_target),
        minimum_effective_target_ticks: ticks(settings.minimum_effective_target),
        lunch_start_minute_of_day: minute_of_day(settings.lunch_start),
        lunch_end_minute_of_day: minute_of_day(settings.lunch_end),
        preferred_latest_departure_minute_of_day: minute_of_day(
            settings.preferred_latest_departure,
        ),
        orange_warning_margin_ticks: ticks(settings.orange_warning_margin),
        long_lock_review_threshold_ticks: ticks(settings.long_lock_review_threshold),
        weekly_credit_enabled: settings.weekly_credit_enabled,
        start_with_windows: settings.start_with_windows,
        notifications: DiagnosticNotificationSettingsSnapshot {
            completion_approach: rule(settings.notifications.completion_approach),
            latest_departure_approach: rule(settings.notifications.latest_departure_approach),
            day_complete: rule(settings.notifications.day_complete),
        },
        activity_watch: DiagnosticActivityWatchTimingSnapshot {
            request_timeout_ticks: ticks(activity_watch.request_timeout),
            afk_timeout_ticks: ticks(activity_watch.settlement.afk_timeout),
            poll_interval_ticks: ticks(activity_watch.settlement.poll_interval),
            maximum_jitter_ticks: ticks(activity_watch.settlement.maximum_jitter),
        },
        logging: DiagnosticLoggingSettingsSnapshot {
            maximum_file_count: settings.logging.maximum_file_count,
            maximum_file_bytes: settings.logging.maximum_file_bytes,
        },
        check_for_updates: settings.updates.check_for_updates,
    }
}

fn summary_snapshot(
    state: &WorkdayRefreshState,
    current_workday: Option<Date>,
    now: Timestamp,
) -> Option<DiagnosticSummarySnapshot> {
    let summary = state.summary.as_ref()?;
    let computed_at = summary.computed_at();
    let health = summary.source_health();
    let is_current = state.source_failure.is_none()
        && current_workday == Some(summary.workday().date)
        && health.freshness == SourceFreshness::Fresh
        && computed_at <= now
        && now.duration_since(computed_at) <= MAXIMUM_CURRENT_SUMMARY_AGE;
    let assessment = summary.assessment();
    Some(DiagnosticSummarySnapshot {
        availability: if is_current {
            DiagnosticSummaryAvailability::Current
        } else {
            DiagnosticSummaryAvailability::Stale
        },
        credited_work_ticks: ticks(summary.credited_work()),
        nominal_target_ticks: ticks(summary.nominal_target()),
        effective_target_ticks: ticks(summary.effective_target()),
        remaining_work_ticks: ticks(summary.remaining_work()),
        credited_overtime_ticks: ticks(summary.credited_overtime()),
        applied_carry_credit_ticks: ticks(summary.applied_carry_credit()),
        indicator: match summary.indicator_state() {
            IndicatorState::Green => DiagnosticIndicator::Green,
            IndicatorState::Orange => DiagnosticIndicator::Orange,
            IndicatorState::Red => DiagnosticIndicator::Red,
            IndicatorState::Blue => DiagnosticIndicator::Blue,
        },
        forecast: match assessment.forecast() {
            ForecastAvailability::Confirmed => DiagnosticForecastAvailability::Confirmed,
            ForecastAvailability::Estimated => DiagnosticForecastAvailability::Estimated,
            ForecastAvailability::Unavailable => DiagnosticForecastAvailability::Unavailable,
        },
        assessment_reasons: assessment
            .reasons()
            .iter()
            .map(|reason| match reason {
                AssessmentReason::PendingSettlement => {
                    DiagnosticAssessmentReason::PendingSettlement
                }
                AssessmentReason::InteriorCompatibilitySeam => {
                    DiagnosticAssessmentReason::InteriorCompatibilitySeam
                }
                AssessmentReason::HistoricalMissingCoverage => {
                    DiagnosticAssessmentReason::HistoricalMissingCoverage
                }
                AssessmentReason::CurrentMissingCoverage => {
                    DiagnosticAssessmentReason::CurrentMissingCoverage
                }
                AssessmentReason::SourceUnavailable => {
                    DiagnosticAssessmentReason::SourceUnavailable
                }
                AssessmentReason::SourceStale => DiagnosticAssessmentReason::SourceStale,
                AssessmentReason::SourceProtocolFailure => {
                    DiagnosticAssessmentReason::SourceProtocolFailure
                }
                AssessmentReason::ProvisionalLockReview => {
                    DiagnosticAssessmentReason::ProvisionalLockReview
                }
            })
            .collect(),
        source_freshness: match health.freshness {
            SourceFreshness::Fresh => DiagnosticSourceFreshness::Fresh,
            SourceFreshness::Stale => DiagnosticSourceFreshness::Stale,
            SourceFreshness::Unavailable => DiagnosticSourceFreshness::Unavailable,
        },
        source_quality: match health.quality {
            SourceQuality::Complete => DiagnosticSourceQuality::Complete,
            SourceQuality::Incomplete => DiagnosticSourceQuality::Incomplete,
            SourceQuality::Ambiguous => DiagnosticSourceQuality::Ambiguous,
            SourceQuality::Unauthorized => DiagnosticSourceQuality::Unauthorized,
            SourceQuality::Incompatible => DiagnosticSourceQuality::Incompatible,
            SourceQuality::Malformed => DiagnosticSourceQuality::Malformed,
        },
    })
}

/// A .NET `TimeSpan` holds at most `i64` ticks, so a longer duration never came from a .NET
/// value; it is written as the nearest one.
fn ticks(duration: SignedDuration) -> i64 {
    to_ticks(duration).unwrap_or(if duration.is_negative() {
        i64::MIN
    } else {
        i64::MAX
    })
}

fn minute_of_day(time: Time) -> i32 {
    i32::from(time.hour()) * 60 + i32::from(time.minute())
}

impl DiagnosticSnapshot {
    /// The bytes of `snapshot.json`.
    pub(crate) fn to_json(&self) -> String {
        JsonObject::write(|root| {
            root.name("initialization", self.initialization.name());
            if let Some(settings) = &self.settings {
                root.object("settings", |object| settings.write(object));
            }
            root.object("runtime", |object| self.runtime.write(object));
            if let Some(summary) = &self.summary {
                root.object("summary", |object| summary.write(object));
            }
        })
    }
}

impl DiagnosticSettingsSnapshot {
    fn write(&self, object: &mut JsonObject) {
        object.value("nominalDailyTargetTicks", self.nominal_daily_target_ticks);
        object.value(
            "minimumEffectiveTargetTicks",
            self.minimum_effective_target_ticks,
        );
        object.value("lunchStartMinuteOfDay", self.lunch_start_minute_of_day);
        object.value("lunchEndMinuteOfDay", self.lunch_end_minute_of_day);
        object.value(
            "preferredLatestDepartureMinuteOfDay",
            self.preferred_latest_departure_minute_of_day,
        );
        object.value("orangeWarningMarginTicks", self.orange_warning_margin_ticks);
        object.value(
            "longLockReviewThresholdTicks",
            self.long_lock_review_threshold_ticks,
        );
        object.value("weeklyCreditEnabled", self.weekly_credit_enabled);
        object.value("startWithWindows", self.start_with_windows);
        object.object("notifications", |notifications| {
            let rules = &self.notifications;
            for (name, rule) in [
                ("completionApproach", rules.completion_approach),
                ("latestDepartureApproach", rules.latest_departure_approach),
                ("dayComplete", rules.day_complete),
            ] {
                notifications.object(name, |object| {
                    object.value("enabled", rule.enabled);
                    object.value("leadTimeTicks", rule.lead_time_ticks);
                });
            }
        });
        object.object("activityWatch", |timing| {
            let activity_watch = &self.activity_watch;
            timing.value("requestTimeoutTicks", activity_watch.request_timeout_ticks);
            timing.value("afkTimeoutTicks", activity_watch.afk_timeout_ticks);
            timing.value("pollIntervalTicks", activity_watch.poll_interval_ticks);
            timing.value("maximumJitterTicks", activity_watch.maximum_jitter_ticks);
        });
        object.object("logging", |logging| {
            logging.value("maximumFileCount", self.logging.maximum_file_count);
            logging.value("maximumFileBytes", self.logging.maximum_file_bytes);
        });
        object.value("checkForUpdates", self.check_for_updates);
    }
}

impl DiagnosticRuntimeSnapshot {
    fn write(&self, object: &mut JsonObject) {
        for (name, version) in [
            ("applicationVersion", self.application_version),
            ("operatingSystemVersion", self.operating_system_version),
            ("dotNetVersion", self.dot_net_version),
        ] {
            object.object(name, |object| {
                object.value("major", version.major);
                object.value("minor", version.minor);
                object.value("build", version.build);
                object.value("revision", version.revision);
            });
        }
        object.name("processArchitecture", self.process_architecture.name());
        object.name(
            "operatingSystemArchitecture",
            self.operating_system_architecture.name(),
        );
    }
}

impl DiagnosticSummarySnapshot {
    fn write(&self, object: &mut JsonObject) {
        object.name("availability", self.availability.name());
        object.value("creditedWorkTicks", self.credited_work_ticks);
        object.value("nominalTargetTicks", self.nominal_target_ticks);
        object.value("effectiveTargetTicks", self.effective_target_ticks);
        object.value("remainingWorkTicks", self.remaining_work_ticks);
        object.value("creditedOvertimeTicks", self.credited_overtime_ticks);
        object.value("appliedCarryCreditTicks", self.applied_carry_credit_ticks);
        object.name("indicator", self.indicator.name());
        object.name("forecast", self.forecast.name());
        object.names(
            "assessmentReasons",
            self.assessment_reasons.iter().map(|reason| reason.name()),
        );
        object.name("sourceFreshness", self.source_freshness.name());
        object.name("sourceQuality", self.source_quality.name());
    }
}

#[cfg(test)]
mod tests;
