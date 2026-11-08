//! The day snapshot a plugin reads (FR-031), built from the daily summary only when a plugin
//! listens, and the structural-change rule that decides when `day.refreshed` is sent (FR-032).

use bientot_domain::accounting::{AccountingLabel, SegmentKind as Accounted, TrackingState};
use bientot_domain::forecasting::{ForecastAvailability, lunch_as_taken, project_departure};
use bientot_domain::intervals::UtcInterval;
use bientot_domain::summaries::DailySummary;
use bientot_plugin_api::Language;
use bientot_plugin_api::snapshot::{
    DaySnapshot, Forecast, OpenTail, SCHEMA, Segment, SegmentKind, Source, Tracking, Week,
};
use jiff::{SignedDuration, Timestamp};

/// What the snapshot is built from: the day's summary and what the app knows beside it.
pub struct DayInput<'a> {
    pub summary: &'a DailySummary,
    pub tracking: TrackingState,
    pub working_day: bool,
    /// The lunch window of the settings for this day.
    pub lunch: UtcInterval,
    /// `minimum_effective_target` of the settings.
    pub minimum: SignedDuration,
    pub language: Language,
}

pub fn build(input: &DayInput) -> DaySnapshot {
    let summary = input.summary;
    let computed_at = summary.computed_at();
    let timeline = summary.accounting().timeline();
    let lunch = lunch_as_taken(input.lunch, timeline);
    let counted = summary.credited_work();
    let target = summary.effective_target();
    let minimum = input.minimum.min(summary.nominal_target());
    let forecast = match summary.assessment().forecast() {
        ForecastAvailability::Confirmed => Forecast::Firm,
        ForecastAvailability::Estimated => Forecast::Estimated,
        ForecastAvailability::Unavailable => Forecast::Unavailable,
    };
    let departure = |target: SignedDuration| {
        if forecast == Forecast::Unavailable || !target.is_positive() {
            return None;
        }
        project_departure(computed_at, counted, target, lunch)
            .ok()
            .flatten()
            .map(|projection| projection.projected_departure)
    };
    let mut segments: Vec<Segment> = timeline
        .segments()
        .iter()
        .map(|segment| Segment {
            start: segment.interval().start(),
            end: segment.interval().end(),
            kind: kind(segment.kind(), segment.label()),
        })
        .collect();
    // The segment that reaches the time of the calculation is still growing.
    let open_tail = match segments.last() {
        Some(last) if last.end >= computed_at => segments.pop().map(|open| OpenTail {
            start: open.start,
            kind: open.kind,
        }),
        _ => None,
    };
    DaySnapshot {
        schema: SCHEMA,
        computed_at,
        time_zone: summary.workday().time_zone_id.clone(),
        date: summary.workday().date,
        language: input.language,
        tracking: match (input.working_day, input.tracking) {
            (false, _) => Tracking::NonWorkingDay,
            (true, TrackingState::Running) => Tracking::Running,
            (true, TrackingState::Paused) => Tracking::Paused,
            (true, TrackingState::Finished) => Tracking::Finished,
        },
        target: target.as_secs(),
        minimum: minimum.as_secs(),
        counted: counted.as_secs(),
        departure_at_target: departure(target),
        departure_at_minimum: departure(minimum),
        forecast,
        segments,
        open_tail,
        week: Week {
            credit: summary
                .credit_result()
                .map_or(0, |credit| credit.net_closing_balance().as_secs()),
        },
        source: if summary.has_source_warning() {
            Source::Degraded
        } else {
            Source::Ok
        },
    }
}

/// The public kind of an accounted segment. Unknown time is `other`, whatever its label.
fn kind(accounted: &Accounted, label: AccountingLabel) -> SegmentKind {
    if *accounted == Accounted::Unknown {
        return SegmentKind::Other;
    }
    match label {
        AccountingLabel::ActiveWork
        | AccountingLabel::RebuiltWork
        | AccountingLabel::LockedWork => SegmentKind::Work,
        AccountingLabel::Afk | AccountingLabel::Suspended | AccountingLabel::OvernightLock => {
            SegmentKind::Away
        }
        AccountingLabel::LunchLock => SegmentKind::Lunch,
        AccountingLabel::ManualPause => SegmentKind::Break,
        AccountingLabel::Manual => SegmentKind::Manual,
        AccountingLabel::ScheduledAway => SegmentKind::Appointment,
        AccountingLabel::DayFinished | AccountingLabel::Unknown => SegmentKind::Other,
    }
}

/// Whether `next` differs from the last snapshot sent in a way a plugin should hear about: a
/// segment settled, the tracking state, the target or the minimum, the forecast, the source, the
/// open tail's kind, the language, or a departure that moved by a minute or more. Counted time
/// that only grows is not a change.
pub fn is_structural(sent: &DaySnapshot, next: &DaySnapshot) -> bool {
    let moved = |a: Option<Timestamp>, b: Option<Timestamp>| match (a, b) {
        (Some(a), Some(b)) => (a.as_second() - b.as_second()).abs() >= 60,
        (a, b) => a.is_some() != b.is_some(),
    };
    sent.date != next.date
        || sent.segments != next.segments
        || sent.tracking != next.tracking
        || sent.target != next.target
        || sent.minimum != next.minimum
        || sent.forecast != next.forecast
        || sent.source != next.source
        || sent.language != next.language
        || sent.open_tail.map(|t| (t.start, t.kind)) != next.open_tail.map(|t| (t.start, t.kind))
        || sent.week != next.week
        || moved(sent.departure_at_target, next.departure_at_target)
        || moved(sent.departure_at_minimum, next.departure_at_minimum)
}

#[cfg(test)]
mod tests;
