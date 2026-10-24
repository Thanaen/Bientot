//! The week read from the ledger, placed on one hour range: each day's figures and segments.

use std::collections::HashMap;

use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use worktime_domain::accounting::{AdjustmentId, LongLockReviewId};
use worktime_domain::configuration::RecurringAppointment;
use worktime_domain::history::{CurrentWeek, CurrentWeekDay};
use worktime_domain::intervals::UtcInterval;
use worktime_domain::time::{LocalWorkday, TimeZoneProvider};

use super::{
    AppointmentMarker, CountedCorrection, Credit, DEFAULT_END_HOUR, DEFAULT_START_HOUR, Lock,
    LockReview, MarkerEnd, Offer, PlannedAppointment, Segment, SegmentId, SegmentKind, details,
    is_correctable,
};
use crate::workday::MINIMUM_REPORTED_MISSING_TIME;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Loaded,
    NoSummary,
    Future,
}

/// What the day's latest summary says, in the zone the day was recorded in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Figures {
    pub workday: LocalWorkday,
    pub credited: SignedDuration,
    pub effective_target: SignedDuration,
    pub nominal_target: SignedDuration,
    pub projected_departure: Option<Time>,
    pub credit: Credit,
    pub corrections: Vec<CountedCorrection>,
}

impl Figures {
    pub(super) fn reached(&self) -> bool {
        self.credited >= self.effective_target
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DayTimeline {
    pub date: Date,
    pub availability: Availability,
    pub figures: Option<Figures>,
    pub segments: Vec<Segment>,
    /// The recurring appointments planned on the day, over the strip.
    pub appointments: Vec<AppointmentMarker>,
    /// Where the « maintenant » line falls, when it is within the drawn hours.
    pub now: Option<f32>,
    /// Today, even when the line falls outside the drawn hours.
    pub is_today: bool,
}

impl DayTimeline {
    pub fn pending_count(&self) -> usize {
        self.segments
            .iter()
            .filter(|segment| segment.needs_action)
            .count()
    }

    /// What the week offers on the correction `id`, as the day counts it.
    pub(super) fn offer_of(&self, id: &AdjustmentId) -> Option<Offer> {
        let corrections = &self.figures.as_ref()?.corrections;
        Some(corrections.iter().find(|counted| counted.id == *id)?.offer)
    }

    /// What the week offers on the correction `segment` is a piece of.
    pub(super) fn offer(&self, segment: &Segment) -> Option<Offer> {
        self.offer_of(segment.adjustment_id.as_ref()?)
    }

    /// « Retirer la correction » on `segment`: a piece of a correction the ledger withdraws.
    pub(super) fn can_remove(&self, segment: &Segment) -> bool {
        matches!(segment.kind, SegmentKind::Manual | SegmentKind::NotWorked)
            && self.offer(segment).is_some_and(Offer::removes)
    }

    pub(super) fn total(&self, kinds: &[SegmentKind]) -> SignedDuration {
        self.segments
            .iter()
            .filter(|segment| kinds.contains(&segment.kind))
            .map(Segment::duration)
            .sum()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeekTimeline {
    pub week_start: Date,
    pub today: Date,
    pub start_hour: i8,
    pub end_hour: i8,
    pub days: Vec<DayTimeline>,
}

/// A segment and the hours it covers on its day, before the week's range is known.
struct Piece {
    segment: Segment,
    from: f64,
    to: f64,
}

impl WeekTimeline {
    /// Places every day of `week` on one hour range: 8 h–19 h, widened to the whole hour when
    /// a segment falls outside it. A day without a summary keeps the zone it would use today.
    /// `reviews` are the locks the week awaits an answer for; a review segment whose lock is
    /// not among them is only drawn. `offers` say what the week offers on each correction its
    /// summaries count; one they do not name is [`Offer::Unknown`]. `confirmed` are the locks
    /// confirmed as work, whose sleep the ledger leaves uncounted. Each of `appointments`
    /// gets a marker on the days it falls on that show some time, up to now.
    pub fn project(
        week: &CurrentWeek,
        reviews: &[LockReview],
        offers: &HashMap<AdjustmentId, Offer>,
        confirmed: &[(AdjustmentId, UtcInterval)],
        appointments: &[RecurringAppointment],
        now: Timestamp,
        zones: &dyn TimeZoneProvider,
    ) -> Self {
        let (_, local) = zones.local();
        let placed: Vec<_> = week
            .days()
            .iter()
            .map(|day| place_day(day, reviews, offers, confirmed, now, zones, &local))
            .collect();
        let drawn: Vec<f64> = placed
            .iter()
            .flat_map(|(_, pieces, _)| pieces.iter().flat_map(|piece| [piece.from, piece.to]))
            .collect();
        let start_hour = drawn
            .iter()
            .copied()
            .reduce(f64::min)
            .map_or(DEFAULT_START_HOUR, |hour| {
                DEFAULT_START_HOUR.min(hour.floor() as i8)
            });
        let end_hour = drawn
            .iter()
            .copied()
            .reduce(f64::max)
            .map_or(DEFAULT_END_HOUR, |hour| {
                DEFAULT_END_HOUR.max(hour.ceil() as i8)
            });
        let span = f64::from(end_hour - start_hour);
        let fraction = |hour: f64| ((hour - f64::from(start_hour)) / span) as f32;
        let days = placed
            .into_iter()
            .map(|(mut day, pieces, now_hour)| {
                // Hours past now are not drawn yet, nor are hours outside the axis.
                let shown = (
                    f64::from(start_hour),
                    now_hour.map_or(f64::from(end_hour), |now| now.min(f64::from(end_hour))),
                );
                day.appointments = markers(
                    appointments
                        .iter()
                        .filter(|appointment| !pieces.is_empty() && appointment.falls_on(day.date)),
                    shown,
                    now_hour.is_some_and(|now| now < f64::from(end_hour)),
                )
                .into_iter()
                .map(|(from, to, marker)| AppointmentMarker {
                    start: fraction(from),
                    length: ((to - from) / span) as f32,
                    ..marker
                })
                .collect();
                day.segments = pieces
                    .into_iter()
                    .map(|piece| Segment {
                        start: fraction(piece.from),
                        length: ((piece.to - piece.from) / span) as f32,
                        ..piece.segment
                    })
                    .collect();
                day.is_today = now_hour.is_some();
                day.now = now_hour
                    .filter(|hour| (f64::from(start_hour)..=f64::from(end_hour)).contains(hour))
                    .map(fraction);
                day
            })
            .collect();
        Self {
            week_start: week.week_start_date(),
            today: now.to_zoned(local).date(),
            start_hour,
            end_hour,
            days,
        }
    }

    pub fn day(&self, date: Date) -> Option<&DayTimeline> {
        self.days.iter().find(|day| day.date == date)
    }

    pub fn segment(&self, id: SegmentId) -> Option<(&DayTimeline, &Segment)> {
        let day = self.day(id.date)?;
        let segment = day
            .segments
            .iter()
            .find(|segment| segment.interval.start() == id.start)?;
        Some((day, segment))
    }

    pub fn pending_count(&self) -> usize {
        self.days.iter().map(DayTimeline::pending_count).sum()
    }

    /// The segments to settle across the week, in the order « Régler les trous » walks them.
    pub fn pending(&self) -> impl Iterator<Item = SegmentId> + '_ {
        self.days.iter().flat_map(|day| {
            day.segments
                .iter()
                .filter(|segment| segment.needs_action)
                .map(|segment| segment.id(day.date))
        })
    }

    /// The day shown open: the one asked for if it is in the week, else today, else the last
    /// day before today.
    pub fn focus(&self, asked: Option<Date>) -> Date {
        asked
            .filter(|date| self.day(*date).is_some())
            .or_else(|| self.day(self.today).map(|day| day.date))
            .or_else(|| {
                self.days
                    .iter()
                    .rev()
                    .find(|day| day.date <= self.today)
                    .map(|day| day.date)
            })
            .unwrap_or(self.week_start)
    }

    /// Whether each of `ids`, and at least one, is still drawn as a correction the user may
    /// withdraw.
    pub(super) fn shows_removable(&self, ids: &[AdjustmentId]) -> bool {
        !ids.is_empty()
            && ids.iter().all(|id| {
                self.days.iter().any(|day| {
                    day.segments.iter().any(|segment| {
                        segment.adjustment_id.as_ref() == Some(id) && day.can_remove(segment)
                    })
                })
            })
    }

    pub(super) fn workday_of(&self, id: &AdjustmentId) -> Option<LocalWorkday> {
        self.days
            .iter()
            .find(|day| {
                day.segments
                    .iter()
                    .any(|segment| segment.adjustment_id.as_ref() == Some(id))
            })
            .and_then(|day| day.figures.as_ref())
            .map(|figures| figures.workday.clone())
    }
}

fn place_day(
    day: &CurrentWeekDay,
    reviews: &[LockReview],
    offers: &HashMap<AdjustmentId, Offer>,
    confirmed: &[(AdjustmentId, UtcInterval)],
    now: Timestamp,
    zones: &dyn TimeZoneProvider,
    local: &TimeZone,
) -> (DayTimeline, Vec<Piece>, Option<f64>) {
    let snapshot = day
        .latest_revision
        .as_ref()
        .map(|revision| revision.snapshot());
    let zone = snapshot
        .and_then(|snapshot| zones.get(&snapshot.workday().time_zone_id).ok())
        .unwrap_or_else(|| local.clone());
    let now_local = now.to_zoned(zone.clone());
    let availability = if now_local.date() < day.date {
        Availability::Future
    } else if snapshot.is_none() {
        Availability::NoSummary
    } else {
        Availability::Loaded
    };
    let now_hour = (now_local.date() == day.date).then(|| hours(now_local.time()));
    let mut timeline = DayTimeline {
        date: day.date,
        availability,
        figures: None,
        segments: Vec::new(),
        appointments: Vec::new(),
        now: None,
        is_today: false,
    };
    let Some(snapshot) = snapshot else {
        return (timeline, Vec::new(), now_hour);
    };
    timeline.figures = Some(Figures {
        workday: snapshot.workday().clone(),
        credited: snapshot.credited_work(),
        effective_target: snapshot.effective_target(),
        nominal_target: snapshot.nominal_target(),
        projected_departure: snapshot
            .projected_departure()
            .map(|departure| departure.to_zoned(zone.clone()).time()),
        credit: Credit::of(snapshot),
        corrections: details::corrections(snapshot, offers, &zone),
    });

    let midnight = |date: Date| {
        date.to_zoned(zone.clone())
            .expect("a summary's day exists in its zone")
            .timestamp()
    };
    let day_end = midnight(day.date.tomorrow().expect("a week day has a next day"));
    let shown = UtcInterval::new(midnight(day.date), day_end).expect("a day has a length");
    let hour_of = |instant: Timestamp| {
        if instant >= day_end {
            24.
        } else {
            hours(instant.to_zoned(zone.clone()).time())
        }
    };
    let local = |instant: Timestamp| instant.to_zoned(zone.clone()).datetime();
    let lock = |id: &LongLockReviewId| {
        reviews
            .iter()
            .find(|review| review.id == *id)
            .map(|review| Lock {
                id: review.id.clone(),
                resolved: review.resolved,
                local_start: local(review.resolved.start()),
                local_end: local(review.resolved.end()),
                finish_at: review
                    .appointment
                    .is_none()
                    .then(|| local(review.locked_episode.start()).time()),
                appointment: review.appointment.clone(),
                weekday: review.weekday,
                is_open: review.is_open,
                gains: review.gains,
            })
    };
    let has_pending_review = snapshot.has_pending_review();
    let mut pieces: Vec<Piece> = snapshot
        .breakdown()
        .entries()
        .iter()
        .flat_map(|entry| {
            let Some(interval) = entry.interval() else {
                return Vec::new();
            };
            let kind = SegmentKind::of(entry, has_pending_review);
            let review = (kind == SegmentKind::Review)
                .then(|| entry.review_id().and_then(lock))
                .flatten();
            // Sleep within a lock confirmed as work stays uncounted, as the lock's answer
            // leaves it: a piece of that answer, with nothing left to settle. Sleep that began
            // before the lock or goes on after it is cut at its edges.
            let mut cuts: Vec<Timestamp> = match kind {
                SegmentKind::Sleep => confirmed
                    .iter()
                    .flat_map(|(_, lock)| [lock.start(), lock.end()])
                    .filter(|cut| interval.start() < *cut && *cut < interval.end())
                    .collect(),
                _ => Vec::new(),
            };
            cuts.sort();
            cuts.dedup();
            let bounds: Vec<Timestamp> = std::iter::once(interval.start())
                .chain(cuts)
                .chain(std::iter::once(interval.end()))
                .collect();
            bounds
                .windows(2)
                .filter_map(|pair| {
                    let interval = UtcInterval::new(pair[0], pair[1]).ok()?;
                    let part = interval.intersect(shown)?;
                    let confirmation = (kind == SegmentKind::Sleep)
                        .then(|| {
                            confirmed.iter().find(|(_, lock)| {
                                lock.start() <= interval.start() && interval.end() <= lock.end()
                            })
                        })
                        .flatten()
                        .map(|(id, _)| id.clone());
                    // A review stays pending however short its pieces: lunch can split one
                    // long lock into two-minute fragments. Only missing time under five
                    // minutes is left alone.
                    let needs_action = review.is_some()
                        || (confirmation.is_none()
                            && interval.duration() >= MINIMUM_REPORTED_MISSING_TIME
                            && is_correctable(entry));
                    Some(Piece {
                        segment: Segment {
                            kind,
                            interval,
                            start: 0.,
                            length: 0.,
                            local_start: local(interval.start()),
                            local_end: local(interval.end()),
                            needs_action,
                            adjustment_id: confirmation.or_else(|| entry.adjustment_id().cloned()),
                            review: review.clone(),
                        },
                        from: hour_of(part.start()),
                        to: hour_of(part.end()),
                    })
                })
                .collect()
        })
        .collect();
    pieces.sort_by_key(|piece| piece.segment.interval.start());

    // Inactivity before arriving and after leaving is not a pause of the workday: drawing it
    // would stretch every day's axis to midnight. Sleep or a night lock there is not a gap to
    // settle either (#136), once the day holds work to arrive at or, after a finished day, to
    // leave from, nor is unknown time before arriving (#305), as the flyout leaves it out
    // (#301). Sleep between two pieces of the day, and unknown time after arriving, stay
    // pending.
    let worked = pieces.iter().any(|piece| {
        matches!(
            piece.segment.kind,
            SegmentKind::Work | SegmentKind::Manual | SegmentKind::Review
        )
    });
    let off_duty = |piece: &Piece, sleep_is_off_duty: bool| {
        piece.segment.kind == SegmentKind::Pause
            || (sleep_is_off_duty && piece.segment.kind == SegmentKind::Sleep)
    };
    let first = pieces.iter().position(|piece| {
        !off_duty(piece, worked) && !(worked && piece.segment.kind == SegmentKind::Gap)
    });
    let last = pieces
        .iter()
        .rposition(|piece| !off_duty(piece, worked && now_hour.is_none()));
    let pieces = match (first, last) {
        (Some(first), Some(last)) if first <= last => pieces.drain(first..=last).collect(),
        _ => Vec::new(),
    };
    (timeline, pieces, now_hour)
}

/// The brackets of `appointments` on a day drawn over the hours `shown`, which end at now when
/// `cut_by_now`, with the hours each spans. Appointments whose hours overlap share one bracket
/// over their union, so that each stays named in its tooltip.
fn markers<'a>(
    appointments: impl Iterator<Item = &'a RecurringAppointment>,
    shown: (f64, f64),
    cut_by_now: bool,
) -> Vec<(f64, f64, AppointmentMarker)> {
    let mut cut: Vec<_> = appointments
        .filter_map(|appointment| {
            let planned = (hours(appointment.start), hours(appointment.end));
            let (from, to) = (planned.0.max(shown.0), planned.1.min(shown.1));
            let end = |planned_here: bool, cut: MarkerEnd| {
                if planned_here {
                    MarkerEnd::Planned
                } else {
                    cut
                }
            };
            (from < to).then(|| {
                (
                    from,
                    to,
                    AppointmentMarker {
                        appointments: vec![PlannedAppointment {
                            name: appointment.name.clone(),
                            planned: (appointment.start, appointment.end),
                        }],
                        start: 0.,
                        length: 0.,
                        left: end(from == planned.0, MarkerEnd::Axis),
                        right: end(
                            to == planned.1,
                            if cut_by_now {
                                MarkerEnd::Now
                            } else {
                                MarkerEnd::Axis
                            },
                        ),
                    },
                )
            })
        })
        .collect();
    cut.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64, AppointmentMarker)> = Vec::new();
    for (from, to, marker) in cut {
        match merged.last_mut() {
            Some((start, end, last)) if from < *end => {
                // On a tie, the one that goes on past the end sets how it is drawn.
                if from == *start && marker.left != MarkerEnd::Planned {
                    last.left = marker.left;
                }
                if to > *end || (to == *end && marker.right != MarkerEnd::Planned) {
                    *end = to;
                    last.right = marker.right;
                }
                last.appointments.extend(marker.appointments);
            }
            _ => merged.push((from, to, marker)),
        }
    }
    merged
}

fn hours(time: Time) -> f64 {
    f64::from(time.hour())
        + f64::from(time.minute()) / 60.
        + (f64::from(time.second()) + f64::from(time.subsec_nanosecond()) / 1e9) / 3600.
}
