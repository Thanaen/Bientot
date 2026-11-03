//! The loupe a click on a cluster opens under the strip: from half an hour to an hour and a half
//! of the open day around the click, wide enough that each short segment of the cluster is
//! clicked on its own, a two-minute gap 27 px wide over half an hour.

use jiff::SignedDuration;
use jiff::civil::{Date, DateTime, Time};

use super::{LABEL_CHARACTER_WIDTH, SegmentRole, SegmentView, TICK_GAP, WeekState};
use crate::strings::Key;
use crate::time_text::minute_range;

/// The width of the loupe's strip, in pixels.
pub const LOUPE_STRIP_WIDTH: f32 = 408.;
/// What the loupe shows beyond the cluster, in minutes, within its shortest and longest spans.
const MARGIN: f32 = 10.;
const SHORTEST: f32 = 30.;
const LONGEST: f32 = 90.;
/// What the loupe's span and start round to, in minutes.
const STEP: f32 = 5.;
/// The steps of the loupe's axis, in minutes: the first that leaves room between its labels.
const TICK_STEPS: [i32; 3] = [10, 15, 30];

/// The hours the loupe shows on `date`, and the cluster it opened on, as fractions of the
/// week's hour range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loupe {
    date: Date,
    from: f32,
    to: f32,
    cluster: (f32, f32),
}

/// The loupe as the window draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct LoupeView {
    pub date: Date,
    /// The hours shown, as fractions of the hour range, outlined on the strip.
    pub start: f32,
    pub length: f32,
    /// « Vue agrandie · 15:02–15:11 », the cluster's hours: the axis shows the margin around it.
    pub title: String,
    /// The segments shown, their `start` and `length` as fractions of the loupe; a filled one
    /// cut by its left edge shows no start time, which it does not show.
    pub segments: Vec<SegmentView>,
    /// The tooltip of each segment, as the strip's: « Trou · 09:40–09:45 · 5 min ».
    pub tips: Vec<String>,
    /// Where each label of its axis is centred, as a fraction of the loupe.
    pub ticks: Vec<(f32, String)>,
    /// The first segment of the cluster, which the keyboard starts on.
    pub first: usize,
}

impl WeekState {
    /// The minutes of the week's hour range.
    fn range_minutes(&self) -> f32 {
        f32::from(self.timeline.end_hour - self.timeline.start_hour) * 60.
    }

    /// Opens the loupe on the cluster from `start` to `end` of the open day, centred on `at`
    /// as far as the whole cluster still shows. The bubble closes.
    pub(super) fn open_loupe(&mut self, start: f32, end: f32, at: f32) {
        let minutes = self.range_minutes();
        if minutes <= 0. {
            return;
        }
        let span = ((((end - start) * minutes + MARGIN) / STEP).ceil() * STEP)
            .clamp(SHORTEST, LONGEST)
            .min(minutes);
        // The whole cluster, five minutes from either edge, when the span holds it.
        let (earliest, latest) = (
            end * minutes + MARGIN / 2. - span,
            start * minutes - MARGIN / 2.,
        );
        let holds = |from: f32| earliest > latest || (earliest..=latest).contains(&from);
        let centred = at * minutes - span / 2.;
        let centred = if earliest <= latest {
            centred.clamp(earliest, latest)
        } else {
            centred
        };
        // On five minutes when the cluster still shows whole, else on the minute.
        let from = [
            (centred / STEP).round() * STEP,
            (centred / STEP).ceil() * STEP,
            (centred / STEP).floor() * STEP,
        ]
        .into_iter()
        .find(|from| holds(*from))
        .unwrap_or_else(|| centred.round())
        .clamp(0., minutes - span);
        self.selection = None;
        self.loupe = Some(Loupe {
            date: self.focused,
            from: from / minutes,
            to: (from + span) / minutes,
            cluster: (start, end),
        });
    }

    pub fn loupe(&self) -> Option<LoupeView> {
        let loupe = self.loupe?;
        let day = self.timeline.day(loupe.date)?;
        let minutes = self.range_minutes();
        let length = loupe.to - loupe.from;
        if minutes <= 0. || length <= 0. {
            return None;
        }
        let shown: Vec<_> = day
            .segments
            .iter()
            .filter(|segment| {
                segment.start < loupe.to && segment.start + segment.length > loupe.from
            })
            .collect();
        let segments = shown
            .iter()
            .map(|segment| {
                let start = segment.start.max(loupe.from);
                let end = (segment.start + segment.length).min(loupe.to);
                let cut = segment.start < loupe.from;
                SegmentView {
                    id: segment.id(day.date),
                    kind: segment.kind,
                    start: (start - loupe.from) / length,
                    length: (end - start) / length,
                    start_time: if cut {
                        String::new()
                    } else {
                        segment.start_time()
                    },
                    word: segment.kind.name(self.language),
                    accessible_name: segment.accessible_name(self.language),
                    role: SegmentRole::Button,
                }
            })
            .collect();
        let tips = shown
            .iter()
            .map(|segment| segment.aim_text(self.language))
            .collect();
        let first = shown
            .iter()
            .position(|segment| segment.start >= loupe.cluster.0)
            .unwrap_or_default();
        let clustered: Vec<_> = shown
            .iter()
            .filter(|segment| {
                segment.start >= loupe.cluster.0
                    && segment.start + segment.length <= loupe.cluster.1 + f32::EPSILON
            })
            .collect();
        let title = match (clustered.first(), clustered.last()) {
            (Some(first), Some(last)) => self.language.format(
                Key::HistoryLoupeTitleFormat,
                &[&minute_range(first.local_start, last.local_end, self.language).0],
            ),
            _ => String::new(),
        };
        // Minutes of the day, from the hour range's start.
        let opening = i32::from(self.timeline.start_hour) * 60;
        let (from, to) = (
            opening + (loupe.from * minutes).round() as i32,
            opening + (loupe.to * minutes).round() as i32,
        );
        let at = |minute: i32| -> Option<DateTime> {
            day.date
                .to_datetime(Time::midnight())
                .checked_add(SignedDuration::from_mins(i64::from(minute)))
                .ok()
        };
        let per_minute = LOUPE_STRIP_WIDTH / (to - from).max(1) as f32;
        // Written as the week's axis writes its hours, whatever the language.
        let label = |time: Time| time.strftime("%H:%M").to_string();
        let widest = label(Time::midnight()).chars().count() as f32 * LABEL_CHARACTER_WIDTH;
        let step = TICK_STEPS
            .into_iter()
            .find(|step| *step as f32 * per_minute >= widest + TICK_GAP)
            .unwrap_or(60);
        let ticks = (from.div_euclid(step) + 1..)
            .map(|tick| tick * step)
            .take_while(|minute| *minute < to)
            .filter_map(|minute| {
                let time = at(minute)?.time();
                Some(((minute - from) as f32 / (to - from) as f32, label(time)))
            })
            .collect();
        Some(LoupeView {
            date: loupe.date,
            start: loupe.from,
            length,
            title,
            segments,
            tips,
            ticks,
            first,
        })
    }

    /// After a reload: the loupe stays on its day while the day still takes corrections.
    pub(super) fn revalidate_loupe(&mut self) {
        if let Some(loupe) = self.loupe
            && (loupe.date != self.focused
                || !self
                    .timeline
                    .day(loupe.date)
                    .is_some_and(|day| day.selectable()))
        {
            self.loupe = None;
        }
    }
}
