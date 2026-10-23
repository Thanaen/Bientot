//! The range the open day's timeline selects: a click selects a segment, then its edges move by
//! dragging them, with the keyboard or in De and À, snapped to the segments' edges and to five
//! minutes. A range that touches a lock to review covers the whole lock, as answering it does.

use jiff::civil::{Date, DateTime, Time};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use worktime_domain::intervals::UtcInterval;

use super::{Availability, DayTimeline, Segment, SegmentId, WeekState, WeekTimeline};

/// What a range snaps to, and the shortest one.
pub const RANGE_STEP: SignedDuration = SignedDuration::from_mins(5);

/// The range selected on a day, which the bubble under it corrects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub date: Date,
    pub range: UtcInterval,
    /// The segment clicked, while the range is still the one its click selected.
    pub anchor: Option<SegmentId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

impl DayTimeline {
    /// The day the timeline draws, from its first segment to its last.
    fn extent(&self) -> Option<UtcInterval> {
        let first = self.segments.first()?;
        let last = self.segments.last()?;
        UtcInterval::new(first.interval.start(), last.interval.end()).ok()
    }

    /// Whether a range may be selected on the day: a working day with a summary drawn. A day
    /// off takes no correction.
    pub(super) fn selectable(&self) -> bool {
        self.availability == Availability::Loaded
            && self.extent().is_some()
            && self
                .figures
                .as_ref()
                .is_some_and(|figures| figures.workday.is_working_day())
    }

    /// Each segment `range` meets, with the part of it in the range.
    pub(super) fn pieces(
        &self,
        range: UtcInterval,
    ) -> impl Iterator<Item = (&Segment, UtcInterval)> + '_ {
        self.segments
            .iter()
            .filter_map(move |segment| Some((segment, segment.interval.intersect(range)?)))
    }

    /// `range` within the day, at least [`RANGE_STEP`] long by moving `moved`, and over the
    /// whole of each lock to review it touches.
    fn normalized(&self, start: Timestamp, end: Timestamp, moved: Edge) -> Option<UtcInterval> {
        let extent = self.extent()?;
        let clamp = |instant: Timestamp| instant.clamp(extent.start(), extent.end());
        let (mut start, mut end) = (clamp(start), clamp(end));
        if end.duration_since(start) < RANGE_STEP {
            match moved {
                Edge::Start => start = clamp(end - RANGE_STEP).min(start),
                Edge::End => end = clamp(start + RANGE_STEP).max(end),
            }
            if end.duration_since(start) < RANGE_STEP {
                (start, end) = (clamp(end - RANGE_STEP), clamp(start + RANGE_STEP));
            }
        }
        let mut range = UtcInterval::new(start, end).ok()?;
        // Covering one lock may reach another.
        loop {
            let covered = self
                .segments
                .iter()
                .filter_map(|segment| segment.review.as_ref())
                .map(|lock| lock.resolved)
                .filter(|resolved| resolved.overlaps(range))
                .fold(range, |range, resolved| {
                    UtcInterval::new(
                        range.start().min(clamp(resolved.start())),
                        range.end().max(clamp(resolved.end())),
                    )
                    .unwrap_or(range)
                });
            if covered == range {
                return Some(range);
            }
            range = covered;
        }
    }

    /// Where `instant` falls on the hour range, within the segment that holds it.
    pub(super) fn fraction(&self, instant: Timestamp) -> Option<f32> {
        let segment = self.segments.iter().find(|segment| {
            segment.interval.end() >= instant && segment.interval.start() <= instant
        })?;
        let into = segment
            .interval
            .start()
            .duration_until(instant)
            .as_secs_f64()
            / segment.duration().as_secs_f64();
        Some(segment.start + segment.length * into as f32)
    }

    /// The instant shown at `local` on this day, with the offset of the segment nearest it.
    fn instant(&self, local: DateTime) -> Option<Timestamp> {
        let nearest = self.segments.iter().min_by_key(|segment| {
            local
                .duration_since(segment.local_start)
                .abs()
                .min(local.duration_since(segment.local_end).abs())
        })?;
        let offset = civil_seconds(nearest.local_start) - nearest.interval.start().as_second();
        Timestamp::from_second(civil_seconds(local) - offset).ok()
    }

    /// The local time `instant` shows on this day.
    pub(super) fn local(&self, instant: Timestamp) -> Option<DateTime> {
        let nearest = self.segments.iter().min_by_key(|segment| {
            segment
                .interval
                .start()
                .duration_until(instant)
                .abs()
                .min(segment.interval.end().duration_until(instant).abs())
        })?;
        let offset = civil_seconds(nearest.local_start) - nearest.interval.start().as_second();
        let local = Timestamp::from_second(instant.as_second() + offset).ok()?;
        Some(local.to_zoned(TimeZone::UTC).datetime())
    }
}

fn civil_seconds(local: DateTime) -> i64 {
    local
        .to_zoned(TimeZone::UTC)
        .map_or(0, |zoned| zoned.timestamp().as_second())
}

/// `instant` on the five-minute grid: the nearest step, or the next one in `direction`.
fn on_step(instant: Timestamp, direction: i64) -> Timestamp {
    let step = RANGE_STEP.as_secs();
    let seconds = instant.as_second();
    let snapped = match direction.signum() {
        0 => (seconds + step / 2).div_euclid(step) * step,
        1 => (seconds.div_euclid(step) + 1) * step,
        _ => (seconds - 1).div_euclid(step) * step,
    };
    Timestamp::from_second(snapped).unwrap_or(instant)
}

impl WeekTimeline {
    /// Where `fraction` of the hour range falls on `day`.
    fn instant_at(&self, day: &DayTimeline, fraction: f32) -> Option<Timestamp> {
        let hours = f64::from(self.start_hour)
            + f64::from(fraction) * f64::from(self.end_hour - self.start_hour);
        let local = day
            .date
            .to_datetime(Time::midnight())
            .checked_add(SignedDuration::from_secs_f64(hours.max(0.) * 3600.))
            .ok()?;
        day.instant(local)
    }
}

impl WeekState {
    pub fn selection(&self) -> Option<Selection> {
        self.selection
    }

    fn selected_day(&self) -> Option<(&DayTimeline, Selection)> {
        let selection = self.selection?;
        Some((self.timeline.day(selection.date)?, selection))
    }

    /// Moves `edge` of the selection to `instant`, as the edge it moved, then keeps the range
    /// valid. The anchor is lost: the range is no longer the segment clicked. A fixed range
    /// does not move.
    fn move_edge(&mut self, edge: Edge, instant: Timestamp) {
        // A range that takes no answer shows as it is, whichever way its edges would move.
        if self.bubble().is_some_and(|bubble| bubble.fixed) {
            return;
        }
        let Some((day, selection)) = self.selected_day() else {
            return;
        };
        let (start, end) = match edge {
            Edge::Start => (instant.min(selection.range.end()), selection.range.end()),
            Edge::End => (
                selection.range.start(),
                instant.max(selection.range.start()),
            ),
        };
        let Some(range) = day.normalized(start, end, edge) else {
            return;
        };
        if range != selection.range {
            self.selection = Some(Selection {
                range,
                anchor: None,
                ..selection
            });
        }
    }

    /// A drag of `edge` to `fraction` of the hour range: onto a segment's edge within
    /// `tolerance`, else onto the nearest five minutes.
    pub fn drag(&mut self, edge: Edge, fraction: f32, tolerance: f32) {
        let Some((day, _)) = self.selected_day() else {
            return;
        };
        let onto_edge = day
            .segments
            .iter()
            .flat_map(|segment| {
                [
                    (segment.start, segment.interval.start()),
                    (segment.start + segment.length, segment.interval.end()),
                ]
            })
            .map(|(at, instant)| ((at - fraction).abs(), instant))
            .filter(|(distance, _)| *distance <= tolerance)
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, instant)| instant);
        let instant =
            onto_edge.or_else(|| Some(on_step(self.timeline.instant_at(day, fraction)?, 0)));
        if let Some(instant) = instant {
            self.move_edge(edge, instant);
        }
    }

    /// Alt+← and Alt+→ on the end, with Maj on the start: five minutes earlier or later.
    pub fn nudge(&mut self, edge: Edge, direction: i64) {
        let Some((_, selection)) = self.selected_day() else {
            return;
        };
        let at = match edge {
            Edge::Start => selection.range.start(),
            Edge::End => selection.range.end(),
        };
        self.move_edge(edge, on_step(at, direction));
    }

    /// Maj+← and Maj+→: the range takes in the segment before its start or after its end.
    pub fn extend(&mut self, edge: Edge) {
        let Some((day, selection)) = self.selected_day() else {
            return;
        };
        let range = selection.range;
        let reached = match edge {
            Edge::Start => day
                .segments
                .iter()
                .rev()
                .map(|segment| segment.interval.start())
                .find(|start| *start < range.start()),
            Edge::End => day
                .segments
                .iter()
                .map(|segment| segment.interval.end())
                .find(|end| *end > range.end()),
        };
        if let Some(reached) = reached {
            self.move_edge(edge, reached);
        }
    }

    /// De or À typed as `time` on the selected day; À on 00:00 is the midnight that closes it.
    pub fn set_time(&mut self, edge: Edge, time: Time) {
        let Some((day, _)) = self.selected_day() else {
            return;
        };
        let local = match edge {
            Edge::End if time == Time::midnight() => crate::time_text::day_close(day.date),
            _ => Some(day.date.to_datetime(time)),
        };
        if let Some(instant) = local.and_then(|local| day.instant(local)) {
            self.move_edge(edge, instant);
        }
    }

    /// ← and → on the open day: the segment before or after the selected one, or the first or
    /// the last when none is.
    pub fn step(&mut self, delta: isize) {
        let Some(day) = self.timeline.day(self.focused) else {
            return;
        };
        let count = day.segments.len();
        let selected = self
            .selection
            .filter(|selection| selection.date == day.date)
            .map(|selection| selection.range);
        // Past the whole range, so that a lock in pieces or a dragged range is stepped over.
        let index = match selected {
            Some(range) if delta < 0 => day
                .segments
                .iter()
                .rposition(|segment| segment.interval.end() <= range.start()),
            Some(range) => day
                .segments
                .iter()
                .position(|segment| segment.interval.start() >= range.end()),
            None if delta < 0 => count.checked_sub(1),
            None => (count > 0).then_some(0),
        };
        if let Some(segment) = index.and_then(|index| day.segments.get(index)) {
            let id = segment.id(day.date);
            self.open(id);
        }
    }

    /// Selects the segment `id`, or the whole lock when it is a piece of one, and opens its
    /// bubble. A segment of a day without a summary is only drawn.
    pub fn open(&mut self, id: SegmentId) {
        let Some((day, segment)) = self.timeline.segment(id) else {
            return;
        };
        if !day.selectable() {
            return;
        }
        let Some(range) =
            day.normalized(segment.interval.start(), segment.interval.end(), Edge::End)
        else {
            return;
        };
        self.focused = id.date;
        self.selection = Some(Selection {
            date: id.date,
            range,
            anchor: Some(id),
        });
    }

    /// After a reload: the range covers again each lock to review it now touches, or closes
    /// with its day. The segment clicked may now be another one starting at the same time: the
    /// range stays, as no longer the segment clicked, unless that segment still selects it.
    pub(super) fn renormalize_selection(&mut self) {
        let Some(selection) = self.selection else {
            return;
        };
        let range = self.timeline.day(selection.date).and_then(|day| {
            day.normalized(selection.range.start(), selection.range.end(), Edge::End)
        });
        let Some(range) = range else {
            self.selection = None;
            return;
        };
        let anchor = selection.anchor.filter(|id| {
            self.timeline.segment(*id).and_then(|(day, segment)| {
                day.normalized(segment.interval.start(), segment.interval.end(), Edge::End)
            }) == Some(range)
        });
        self.selection = Some(Selection {
            range,
            anchor,
            ..selection
        });
    }

    /// Escape: the selection and its bubble close.
    pub fn close_bubble(&mut self) -> bool {
        self.selection.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_second(seconds).unwrap()
    }

    #[test]
    fn a_step_rounds_to_the_nearest_five_minutes_or_moves_past_it() {
        assert_eq!(on_step(at(149), 0), at(0));
        assert_eq!(on_step(at(150), 0), at(300));
        assert_eq!(on_step(at(300), 1), at(600));
        assert_eq!(on_step(at(301), 1), at(600));
        assert_eq!(on_step(at(300), -1), at(0));
        assert_eq!(on_step(at(299), -1), at(0));
    }
}
