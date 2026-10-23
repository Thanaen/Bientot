//! What the flyout and the tray tooltip say about the current day, presented from the state a
//! refresh publishes. Ported from `Presentation/Workday/WorkdayFlyoutViewModel.cs` and
//! `SourceDiagnosticsPresenter.cs`, laid out for the GPUI flyout.

use std::cmp::Ordering;

use gpui::SharedString;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use worktime_core::routing::{ACTIVITY_WATCH_ID, NO_SOURCE_ID};
use worktime_core::weekly::WorkdayRefreshState;
use worktime_domain::accounting::{SegmentKind, TrackingState};
use worktime_domain::configuration::WorkTimeSettings;
use worktime_domain::forecasting::{AssessmentReason, ForecastAvailability};
use worktime_domain::intervals::{IntervalSet, UtcInterval};
use worktime_domain::sources::{SourceFreshness, SourceHealth, SourceIntervalKind, SourceQuality};
use worktime_domain::summaries::{DailySummary, IndicatorState};
use worktime_windows::activity::BuiltInActivitySource;

use crate::adjust::{Stretch, Unrecorded};
use crate::strings::{Key, Language, keep_together};
use crate::tray_model::ClosedReview;
use crate::week;

/// Missing time shorter than this is not reported: not in the flyout's source hint, as
/// `SourceDiagnosticsPresenter.MinimumReportedMissingTime` of the .NET app, nor offered for
/// settling in the week (#84).
pub(crate) const MINIMUM_REPORTED_MISSING_TIME: SignedDuration = SignedDuration::from_mins(5);

/// The colour and icon of the day's status. A source problem and a near limit are both
/// warnings, but must not look alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    OnTrack,
    NearLimit,
    Late,
    Complete,
    NonWorkingDay,
    SourceProblem,
    /// Tracking paused on a working day.
    Paused,
    /// Tracking finished on a working day.
    Finished,
}

/// The colour of the notification-area icon, `TrayIndicator` in the .NET app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayColour {
    Green,
    Orange,
    Red,
    Blue,
}

/// The notification-area icon: the day's colour, with a badge while the source is in trouble.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrayIcon {
    pub colour: TrayColour,
    pub degraded: bool,
}

impl TrayIcon {
    /// The asset's name, as `TrayIconAssets.GetUri` writes it.
    pub fn file_stem(self) -> &'static str {
        match (self.colour, self.degraded) {
            (TrayColour::Green, false) => "indicator-green",
            (TrayColour::Orange, false) => "indicator-orange",
            (TrayColour::Red, false) => "indicator-red",
            (TrayColour::Blue, false) => "indicator-blue",
            (TrayColour::Green, true) => "indicator-green-degraded",
            (TrayColour::Orange, true) => "indicator-orange-degraded",
            (TrayColour::Red, true) => "indicator-red-degraded",
            (TrayColour::Blue, true) => "indicator-blue-degraded",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Headline {
    pub label: Option<SharedString>,
    pub value: SharedString,
    /// A departure time or a duration, shown large; a message otherwise.
    pub emphasized: bool,
    pub hint: Option<SharedString>,
    /// Until the lunch window ends, that the departure leaves all of it free.
    pub lunch: Option<SharedString>,
    /// How far the forecast can be trusted, when it is only estimated.
    pub qualifier: Option<SharedString>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub tone: Tone,
    pub text: SharedString,
}

/// What replaces the day's status and headline while tracking is paused or finished. A source in
/// trouble keeps its status rather than the tracking state, and resuming brings no forecast back
/// then. A finished day gives the gap from its worked row to its target, but while the source is
/// in trouble that gap is unknown, so it points to adjusting the time instead.
#[derive(Clone, Debug, PartialEq)]
pub struct Stopped {
    pub paused: Replaced,
    pub finished: Replaced,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Replaced {
    pub status: Status,
    pub headline: Headline,
}

/// What a status bar says about tracking, as `TrackingText` of the .NET app: the tracking
/// state on a working day, that no work is scheduled on a day off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackingStatus {
    Tracking(TrackingState),
    DayOff,
}

/// The status and headline the flyout shows for one tracking state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown<'a> {
    pub tone: Tone,
    pub status: &'a SharedString,
    pub headline: &'a Headline,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Worked {
    pub label: SharedString,
    pub worked: SharedString,
    /// `None` on a day off: no work is scheduled, so the time worked counts toward nothing.
    pub target: Option<Target>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub duration: SharedString,
    /// Share of the target already worked, from 0 to 1.
    pub progress: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Credit {
    pub label: SharedString,
    pub value: SharedString,
}

/// What the flyout says about the activity source, after `SourceHintKind` of the .NET app.
#[derive(Clone, Debug, PartialEq)]
pub enum SourceHint {
    /// `OperationalFailure`: a warning, which a refresh may end.
    Failure {
        message: SharedString,
        remedy: FailureRemedy,
    },
    /// `MissingCoverage`: time the source could not record is still unknown. Only information,
    /// with a way to record the work done then.
    Unrecorded {
        message: SharedString,
        /// The unknown time, merged.
        intervals: Vec<UtcInterval>,
        /// The day from arrival, which a declaration shows the unknown time in.
        day: Vec<Stretch>,
    },
}

/// What a failed source offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureRemedy {
    /// The problem is ActivityWatch's, which v1 no longer reads: the built-in detection.
    UseBuiltIn,
    /// Its settings, where the .NET app sends an operational failure.
    OpenSettings,
}

impl SourceHint {
    pub fn message(&self) -> &SharedString {
        match self {
            Self::Failure { message, .. } | Self::Unrecorded { message, .. } => message,
        }
    }

    /// What a declaration of work opens on: every interval of the unrecorded time, in its day.
    /// The .NET app opened on the time only when it was one interval, and on 09:00–10:00
    /// otherwise.
    pub fn declaration(&self) -> Option<Unrecorded> {
        match self {
            Self::Unrecorded { intervals, day, .. } => {
                // A fragment under a minute would be a chip « 10:42–10:42 »; it is left to the
                // strip, unless the time is all fragments.
                let slots: Vec<UtcInterval> = intervals
                    .iter()
                    .copied()
                    .filter(|slot| slot.duration() >= SignedDuration::from_mins(1))
                    .collect();
                let slots = if slots.is_empty() {
                    intervals.clone()
                } else {
                    slots
                };
                Unrecorded::new(slots, day.clone())
            }
            Self::Failure { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DayView {
    pub tone: Tone,
    pub status: SharedString,
    pub headline: Headline,
    /// `None` on a day off and until a refresh produced a summary.
    pub stopped: Option<Stopped>,
    /// `None` until a refresh produced a summary.
    pub worked: Option<Worked>,
    pub credit: Option<Credit>,
    /// What to do about the activity source, when it reported a problem or left time
    /// unrecorded.
    pub source_hint: Option<SourceHint>,
    /// The first absence in a recurring appointment that ended and awaits an answer: the
    /// flyout asks about it. Set by the host, which knows the reviews.
    pub appointment_review: Option<ClosedReview>,
    /// An absence counted automatically, until the user closed its callout.
    pub counted_absence: Option<crate::automatic::CountedAbsence>,
    /// « Compter automatiquement à l’avenir », after confirmations in a row.
    pub trust_offer: Option<crate::automatic::TrustOffer>,
    pub tray_tooltip: SharedString,
    pub tray_icon: TrayIcon,
}

impl DayView {
    /// The status and headline the flyout shows while tracking is `tracking`.
    pub fn shown_while(&self, tracking: TrackingState) -> Shown<'_> {
        let replaced = match (tracking, &self.stopped) {
            (TrackingState::Paused, Some(stopped)) => &stopped.paused,
            (TrackingState::Finished, Some(stopped)) => &stopped.finished,
            _ => {
                return Shown {
                    tone: self.tone,
                    status: &self.status,
                    headline: &self.headline,
                };
            }
        };
        Shown {
            tone: replaced.status.tone,
            status: &replaced.status.text,
            headline: &replaced.headline,
        }
    }

    /// `zone` is that of the refresh; `settings` give the latest departure and the lunch end.
    pub fn present(
        state: &WorkdayRefreshState,
        settings: &WorkTimeSettings,
        zone: &TimeZone,
        language: Language,
    ) -> Self {
        let text = |key| SharedString::from(language.text(key));

        let health = match (&state.source_failure, &state.summary) {
            (Some(failure), _) => Some(failure.diagnostics.operation()),
            (None, Some(summary)) => Some(summary.source_health()),
            (None, None) => None,
        };
        let explicit_failure = state.source_failure.is_some()
            || state
                .summary
                .as_ref()
                .is_some_and(is_explicit_source_failure);
        let activity_watch = health.is_some_and(|health| health.source_id == ACTIVITY_WATCH_ID);
        // `failed`: the source failed even when its health does not say how.
        let source_failure = |failed: bool| {
            let unexplained = if activity_watch {
                Key::SourceActivityWatchIncomplete
            } else {
                Key::SourceIncomplete
            };
            let message = health
                .and_then(source_message)
                .or(failed.then_some(unexplained))?;
            Some(SourceHint::Failure {
                message: text(message),
                remedy: if activity_watch {
                    FailureRemedy::UseBuiltIn
                } else {
                    FailureRemedy::OpenSettings
                },
            })
        };
        let tray_source_failure = || -> SharedString {
            language
                .format(
                    Key::TraySourceFailureFormat,
                    &[&language.text(health.map_or(Key::TraySourceIncomplete, tray_source_cause))],
                )
                .into()
        };

        let Some(summary) = state.summary.as_ref() else {
            return Self {
                tone: Tone::SourceProblem,
                status: text(Key::WorkdayStatusDegraded),
                headline: degraded_headline(language),
                stopped: None,
                worked: None,
                credit: None,
                source_hint: source_failure(true),
                appointment_review: None,
                counted_absence: None,
                trust_offer: None,
                tray_tooltip: tray_source_failure(),
                // `WorkdayRefreshPresenter.Present` of the .NET app, when no summary is left.
                tray_icon: TrayIcon {
                    colour: TrayColour::Green,
                    degraded: true,
                },
            };
        };

        let duration = |duration| flyout_duration(duration, language);
        let degraded = state.source_failure.is_some() || !summary.can_show_forecast();
        // A source in trouble adds its badge to the day's colour.
        let tray_icon = TrayIcon {
            colour: match summary.indicator_state() {
                IndicatorState::Green => TrayColour::Green,
                IndicatorState::Orange => TrayColour::Orange,
                IndicatorState::Red => TrayColour::Red,
                IndicatorState::Blue => TrayColour::Blue,
            },
            degraded: explicit_failure || summary.has_source_warning(),
        };
        let qualifier = (summary.assessment().forecast() == ForecastAvailability::Estimated)
            .then(|| text(Key::WorkdayEstimatedProjectionQualifier));

        let (tone, status) = if !summary.workday().is_working_day() {
            (Tone::NonWorkingDay, Key::WorkdayNonWorkingDayStatus)
        } else if degraded {
            (Tone::SourceProblem, Key::WorkdayStatusDegraded)
        } else {
            match summary.indicator_state() {
                IndicatorState::Green => (Tone::OnTrack, Key::WorkdayStatusGreen),
                IndicatorState::Orange => (Tone::NearLimit, Key::FlyoutStatusNearLimit),
                IndicatorState::Red => (Tone::Late, Key::FlyoutStatusLate),
                IndicatorState::Blue => (Tone::Complete, Key::WorkdayStatusBlue),
            }
        };

        let (headline, tray_primary) = if !summary.workday().is_working_day() {
            let message = text(Key::WorkdayNonWorkingDayPrimaryMessage);
            (message_headline(None, message.clone(), None), message)
        } else if degraded {
            (degraded_headline(language), SharedString::default())
        } else if summary.is_complete() {
            let overtime = duration(summary.credited_overtime());
            let headline = Headline {
                label: Some(text(Key::FlyoutOvertimeLabel)),
                value: overtime.clone(),
                emphasized: true,
                hint: summary
                    .credit_result()
                    .is_none_or(|credit| {
                        credit.earned_transferable_credit().is_positive()
                            && credit.credit_expiring_after_friday().is_zero()
                    })
                    .then(|| text(Key::FlyoutOvertimeHint)),
                lunch: None,
                qualifier: None,
            };
            let tray = language.format(Key::TrayOvertimeFormat, &[&overtime]);
            (headline, tray.into())
        } else {
            let projection = summary
                .departure_projection()
                .expect("an incomplete day has a departure projection");
            let departure = projection.projected_departure;
            let latest = summary
                .workday()
                .date
                .to_datetime(settings.preferred_latest_departure)
                .to_zoned(zone.clone())
                .map_or(departure, |latest| latest.timestamp());
            let margin = latest.duration_since(departure);
            let hint = language.format(
                if margin.is_negative() {
                    Key::FlyoutOverrunHintFormat
                } else {
                    Key::FlyoutMarginHintFormat
                },
                &[
                    &duration(margin.abs()),
                    &language.short_time(latest.to_zoned(zone.clone()).time()),
                ],
            );
            let value = language.short_time(departure.to_zoned(zone.clone()).time());
            let tray = language.format(Key::TrayProjectionFormat, &[&value]);
            let headline = Headline {
                label: Some(text(Key::WorkdayProjectionLabel)),
                value: value.into(),
                emphasized: true,
                hint: Some(hint.into()),
                lunch: (!projection.lunch_over).then(|| {
                    language
                        .format(
                            Key::FlyoutLunchPlannedFormat,
                            &[&language.short_time(settings.lunch_end)],
                        )
                        .into()
                }),
                qualifier,
            };
            (headline, tray.into())
        };

        let worked = Worked {
            label: text(if degraded {
                Key::WorkdayKnownWorkedLabel
            } else {
                Key::WorkdayFlyoutWorkedLabelText
            }),
            worked: duration(summary.credited_work()),
            // The .NET app shows a target on a day off too, though nothing is due then.
            target: summary.workday().is_working_day().then(|| Target {
                duration: duration(summary.effective_target()),
                progress: (summary.credited_work().as_secs_f64()
                    / summary.effective_target().as_secs_f64())
                .clamp(0., 1.) as f32,
            }),
        };
        let balance = summary
            .credit_result()
            .map(|credit| credit.net_closing_balance())
            // A week with neither debt nor credit has nothing to say.
            .filter(|balance| !balance.is_zero());
        let in_debt = balance.is_some_and(|balance| balance.is_negative());
        let credit = balance.map(|balance| {
            if in_debt {
                Credit {
                    label: text(Key::WorkdayFlyoutCreditDebtLabelText),
                    value: duration(balance.abs()),
                }
            } else {
                Credit {
                    label: text(Key::WorkdayFlyoutCreditAvailableLabelText),
                    value: credit_duration(balance, language),
                }
            }
        });

        let tray_tooltip = if degraded {
            tray_source_failure()
        } else {
            let tray_status = match tone {
                Tone::NearLimit => Key::TrayStatusOrange,
                Tone::Late => Key::TrayStatusRed,
                _ => status,
            };
            let primary = match &headline.qualifier {
                Some(qualifier) => format!("{tray_primary} · {qualifier}"),
                None => tray_primary.to_string(),
            };
            let mut tooltip = match &worked.target {
                Some(target) => language.format(
                    Key::WorkdayTooltipFormat,
                    &[
                        &language.text(tray_status),
                        &worked.worked,
                        &target.duration,
                        &primary,
                    ],
                ),
                None => language.format(
                    Key::WorkdayTooltipNoTargetFormat,
                    &[&language.text(tray_status), &worked.worked, &primary],
                ),
            };
            if let Some(debt) = credit.as_ref().filter(|_| in_debt) {
                tooltip = format!(
                    "{tooltip} · {}",
                    language.format(Key::WorkdayDebtFormat, &[&debt.value])
                );
            }
            tooltip.into()
        };

        Self {
            tone,
            status: text(status),
            headline,
            stopped: summary.workday().is_working_day().then(|| {
                // A source problem keeps the header: the first button says the tracking state.
                let replaced = |state, key, message| {
                    let (tone, key) = if degraded {
                        (tone, status)
                    } else {
                        (state, key)
                    };
                    Replaced {
                        status: Status {
                            tone,
                            text: text(key),
                        },
                        headline: message_headline(None, message, None),
                    }
                };
                let (paused, finished) = if degraded {
                    (
                        text(Key::ShellForecastAwaitsSource),
                        text(Key::ShellFinishedNeedsAdjust),
                    )
                } else {
                    (
                        text(Key::ShellResumeForecast),
                        finished_headline(summary, language),
                    )
                };
                Stopped {
                    paused: replaced(Tone::Paused, Key::FlyoutPaused, paused),
                    finished: replaced(Tone::Finished, Key::FlyoutFinished, finished),
                }
            }),
            worked: Some(worked),
            credit,
            source_hint: source_failure(explicit_failure)
                .or_else(|| missing_work(summary, zone, language)),
            appointment_review: None,
            counted_absence: None,
            trust_offer: None,
            tray_tooltip,
            tray_icon,
        }
    }
}

fn message_headline(
    label: Option<SharedString>,
    message: SharedString,
    hint: Option<SharedString>,
) -> Headline {
    Headline {
        label,
        value: message,
        emphasized: false,
        hint,
        lunch: None,
        qualifier: None,
    }
}

/// How far the worked row of a finished day ends from its target.
fn finished_headline(summary: &DailySummary, language: Language) -> SharedString {
    // In whole minutes as the worked row shows them, so the gap matches its arithmetic.
    let gap = summary.credited_work().as_mins() - summary.effective_target().as_mins();
    let gap_format = |key| {
        let gap = flyout_duration(SignedDuration::from_mins(gap.abs()), language);
        language.format(key, &[&gap]).into()
    };
    match gap.cmp(&0) {
        Ordering::Greater => gap_format(Key::ShellFinishedOverFormat),
        Ordering::Equal => language.text(Key::ShellFinishedTargetReached).into(),
        Ordering::Less => gap_format(Key::ShellFinishedShortFormat),
    }
}

fn degraded_headline(language: Language) -> Headline {
    message_headline(
        Some(language.text(Key::WorkdayProjectionLabel).into()),
        language.text(Key::WorkdayDegradedPrimaryMessage).into(),
        Some(language.text(Key::FlyoutDegradedHint).into()),
    )
}

/// Hours and minutes, or minutes alone under an hour, never split across lines.
pub(crate) fn flyout_duration(duration: SignedDuration, language: Language) -> SharedString {
    let minutes = duration.as_mins();
    let text = if minutes < 60 {
        language.format(Key::FlyoutMinutesFormat, &[&minutes])
    } else {
        language.format(
            Key::WorkdayDurationFormat,
            &[&(minutes / 60), &(minutes % 60)],
        )
    };
    keep_together(&text).into()
}

fn credit_duration(duration: SignedDuration, language: Language) -> SharedString {
    if duration.is_positive() && duration < SignedDuration::from_mins(1) {
        keep_together(language.text(Key::WorkdayCreditLessThanMinute)).into()
    } else {
        flyout_duration(duration, language)
    }
}

/// The time the source could not record that the accounting still counts as unknown, as
/// `SourceDiagnosticsPresenter.Present` of the .NET app reports it: nothing under five minutes.
/// Unlike the .NET app, unknown time before the day's first work is left out (#301).
fn missing_work(summary: &DailySummary, zone: &TimeZone, language: Language) -> Option<SourceHint> {
    let segments = summary.accounting().timeline().segments();
    // Unknown time before the first credited work is the night before arrival, not work to
    // declare. Segments are sorted and disjoint, so none straddles arrival. A day without work
    // reports all of it.
    let arrival = segments
        .iter()
        .find(|segment| segment.kind().is_credit())
        .map(|segment| segment.interval().start());
    let unknown = IntervalSet::new(
        segments
            .iter()
            .filter(|segment| *segment.kind() == SegmentKind::Unknown)
            .filter(|segment| arrival.is_none_or(|arrival| segment.interval().start() >= arrival))
            .map(|segment| segment.interval()),
    );
    let missing = IntervalSet::new(
        summary
            .source_diagnostics()
            .intervals()
            .iter()
            .filter(|finding| finding.kind() == SourceIntervalKind::UnresolvedCoverage)
            .flat_map(|finding| {
                let bounds = finding.interval();
                unknown
                    .iter()
                    .filter_map(move |interval| interval.intersect(bounds))
            }),
    );
    let total = missing.covered_duration();
    if total < MINIMUM_REPORTED_MISSING_TIME {
        return None;
    }
    let accounting = summary.accounting();
    let day: Vec<Stretch> = segments
        .iter()
        .filter(|segment| arrival.is_none_or(|arrival| segment.interval().start() >= arrival))
        .map(|segment| Stretch {
            kind: week::SegmentKind::of_accounted(segment, accounting),
            interval: segment.interval(),
        })
        .collect();
    // To the nearest minute, a total read at a glance: « 26 min », not « 25 min 30 s ».
    let duration = crate::time_text::minutes_text(total, language);
    let intervals: Vec<UtcInterval> = missing.iter().copied().collect();
    let clock = |instant: Timestamp| language.short_time(instant.to_zoned(zone.clone()).time());
    let (first, last) = (intervals[0], intervals[intervals.len() - 1]);
    let key = if intervals.len() == 1 {
        Key::SourceMissingOneFormat
    } else {
        Key::SourceMissingSeveralFormat
    };
    Some(SourceHint::Unrecorded {
        message: language
            .format(
                key,
                &[
                    &intervals.len(),
                    &duration,
                    &clock(first.start()),
                    &clock(last.end()),
                ],
            )
            .into(),
        intervals,
        day,
    })
}

fn is_explicit_source_failure(summary: &DailySummary) -> bool {
    let assessment = summary.assessment();
    assessment.forecast() == ForecastAvailability::Unavailable
        && assessment
            .reasons()
            .contains(&AssessmentReason::SourceProtocolFailure)
}

fn is_built_in(health: &SourceHealth) -> bool {
    health.source_id == BuiltInActivitySource::SOURCE_ID
}

/// What the user can do about the source, or `None` when it is healthy. ActivityWatch guidance
/// is given only for ActivityWatch health, as in the .NET app.
fn source_message(health: &SourceHealth) -> Option<Key> {
    use SourceFreshness::{Fresh, Stale, Unavailable};
    use SourceQuality::{Ambiguous, Complete, Incompatible, Incomplete, Malformed, Unauthorized};
    Some(match (health.freshness, health.quality) {
        (Fresh, Complete) if is_built_in(health) => return None,
        (Unavailable, _) if is_built_in(health) => Key::SourceBuiltInInputFailed,
        _ if is_built_in(health) => Key::SourceBuiltInInterrupted,
        _ if health.source_id == NO_SOURCE_ID => Key::SourceNotSelected,
        (_, Unauthorized) => Key::SourceUnauthorized,
        (_, Ambiguous) => Key::SourceAmbiguous,
        (_, Malformed | Incompatible) => Key::SourceMalformed,
        (Unavailable, _) => Key::SourceUnavailable,
        (Stale, _) => Key::SourceStale,
        (_, Incomplete) => Key::SourceActivityWatchIncomplete,
        (Fresh, Complete) => return None,
    })
}

/// The cause the tray tooltip names in `TraySourceFailureFormat`.
fn tray_source_cause(health: &SourceHealth) -> Key {
    use SourceFreshness::{Stale, Unavailable};
    use SourceQuality::{Ambiguous, Incompatible, Malformed, Unauthorized};
    match (health.freshness, health.quality) {
        (Unavailable, _) if is_built_in(health) => Key::TraySourceBuiltInInputFailed,
        _ if is_built_in(health) => Key::TraySourceBuiltInInterrupted,
        _ if health.source_id == NO_SOURCE_ID => Key::TraySourceNotSelected,
        (_, Unauthorized) => Key::TraySourceUnauthorized,
        (_, Ambiguous) => Key::TraySourceAmbiguous,
        (_, Malformed | Incompatible) => Key::TraySourceMalformed,
        (Unavailable, _) => Key::TraySourceUnavailable,
        (Stale, _) => Key::TraySourceStale,
        _ => Key::TraySourceIncomplete,
    }
}

#[cfg(test)]
mod tests;
