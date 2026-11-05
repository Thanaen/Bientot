//! What the pointer aims at on the open day's strip. A segment narrower than [`AIM_WIDTH`] is
//! aimed on that width centred on it, taken from its longer neighbours, and the nearest centre
//! wins among such short ones. Short segments side by side whose centres are closer than that
//! make a cluster: rather than guess which one was meant, a click shows them in the loupe.

use std::ops::Range;

use bientot_domain::intervals::UtcInterval;

use super::{Segment, SegmentId, WeekState, place};
use crate::strings::{Key, Language};
use crate::time_text::minute_range;

/// The narrowest a segment is aimed at, in pixels.
pub const AIM_WIDTH: f32 = 24.;

/// What a click at the pointer does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Selects the segment and opens its bubble.
    Segment(SegmentId),
    /// Shows the short segments of `span` in the loupe.
    Cluster(UtcInterval),
}

/// What the pointer aims at, outlined over the strip with a tooltip under it.
#[derive(Clone, Debug, PartialEq)]
pub struct Aimed {
    pub target: Target,
    /// The outline, as fractions of the hour range.
    pub start: f32,
    pub length: f32,
    /// « Trou · 15:02–15:04 · 2 min », « 3 segments serrés · 15:02–15:11 ».
    pub text: String,
    /// « Cliquez pour agrandir », over a cluster.
    pub hint: Option<&'static str>,
}

/// The segment aimed at `x` on a strip `width` pixels wide where `spans` are drawn, as each
/// one's start and length on the hour range, with the short segments it is clustered with:
/// itself alone when it is long, or stands apart.
pub fn aim(spans: &[(f32, f32)], width: f32, x: f32) -> Option<(usize, Range<usize>)> {
    let placed: Vec<(f32, f32)> = spans
        .iter()
        .map(|&(start, length)| place(start, length, width))
        .collect();
    let short = |index: usize| placed[index].1 < AIM_WIDTH;
    let centre = |index: usize| placed[index].0 + placed[index].1 / 2.;
    let nearest_short = (0..placed.len())
        .filter(|&index| short(index) && (x - centre(index)).abs() <= AIM_WIDTH / 2.)
        .min_by(|&left, &right| {
            (x - centre(left))
                .abs()
                .total_cmp(&(x - centre(right)).abs())
        });
    // Else the segment drawn under the pointer; the last one drawn shows over the others.
    let index = nearest_short.or_else(|| {
        (0..placed.len())
            .rev()
            .find(|&index| placed[index].0 <= x && x <= placed[index].0 + placed[index].1)
    })?;
    let mut cluster = index..index + 1;
    if short(index) {
        while cluster.start > 0
            && short(cluster.start - 1)
            && centre(cluster.start) - centre(cluster.start - 1) < AIM_WIDTH
        {
            cluster.start -= 1;
        }
        while cluster.end < placed.len()
            && short(cluster.end)
            && centre(cluster.end) - centre(cluster.end - 1) < AIM_WIDTH
        {
            cluster.end += 1;
        }
    }
    Some((index, cluster))
}

impl Segment {
    /// « Trou · 15:02–15:04 · 2 min », the tooltip of the segment aimed at.
    pub(super) fn aim_text(&self, language: Language) -> String {
        let (range, duration) = minute_range(self.local_start, self.local_end, language);
        language.format(
            Key::TimelineAimFormat,
            &[&self.kind_name(language), &range, &duration],
        )
    }
}

impl WeekState {
    /// What the pointer at `x` aims at on the open day's strip, `width` pixels wide: nothing on
    /// a day that takes no correction, nor while the loupe is open.
    pub fn aim(&self, x: f32, width: f32) -> Option<Aimed> {
        if self.loupe.is_some() {
            return None;
        }
        let day = self
            .timeline
            .day(self.focused)
            .filter(|day| day.selectable())?;
        let spans: Vec<_> = day
            .segments
            .iter()
            .map(|segment| (segment.start, segment.length))
            .collect();
        let (index, cluster) = aim(&spans, width, x)?;
        if cluster.len() > 1 {
            let (first, last) = (&day.segments[cluster.start], &day.segments[cluster.end - 1]);
            let span = UtcInterval::new(first.interval.start(), last.interval.end()).ok()?;
            let range = minute_range(first.local_start, last.local_end, self.language).0;
            return Some(Aimed {
                target: Target::Cluster(span),
                start: first.start,
                length: last.start + last.length - first.start,
                text: self
                    .language
                    .format(Key::TimelineClusterFormat, &[&cluster.len(), &range]),
                hint: Some(self.language.text(Key::TimelineClusterHint)),
            });
        }
        let segment = &day.segments[index];
        Some(Aimed {
            target: Target::Segment(segment.id(day.date)),
            start: segment.start,
            length: segment.length,
            text: segment.aim_text(self.language),
            hint: None,
        })
    }

    /// A click at `x` on the open day's strip, `width` pixels wide: the segment aimed at opens
    /// its bubble, or closes it when it holds it open; a cluster opens the loupe there. Beside
    /// every segment, or under the open loupe, it closes what is open, as a click anywhere else
    /// in the window does.
    pub fn click(&mut self, x: f32, width: f32) {
        let Some(aimed) = self.aim(x, width) else {
            self.close_bubble();
            return;
        };
        match aimed.target {
            Target::Segment(id) if self.holder() == Some(id) => {
                self.close_bubble();
            }
            Target::Segment(id) => self.open(id),
            Target::Cluster(_) => {
                self.open_loupe(aimed.start, aimed.start + aimed.length, x / width);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On a strip 200 px wide, a four-pixel segment at 100 px between two long ones.
    const LONELY: [(f32, f32); 3] = [(0., 0.5), (0.5, 0.02), (0.52, 0.48)];

    #[test]
    fn a_short_segment_takes_24_pixels_from_its_long_neighbours() {
        assert_eq!(aim(&LONELY, 200., 90.), Some((1, 1..2)));
        assert_eq!(aim(&LONELY, 200., 114.), Some((1, 1..2)));
        assert_eq!(aim(&LONELY, 200., 89.), Some((0, 0..1)));
        assert_eq!(aim(&LONELY, 200., 115.), Some((2, 2..3)));
        assert_eq!(aim(&LONELY, 200., 201.), None);
    }

    #[test]
    fn short_segments_closer_than_24_pixels_make_a_cluster() {
        let side_by_side = [(0., 0.5), (0.5, 0.02), (0.52, 0.02), (0.54, 0.46)];
        // The nearest centre wins, the cluster holds both.
        assert_eq!(aim(&side_by_side, 200., 105.), Some((2, 1..3)));
        assert_eq!(aim(&side_by_side, 200., 103.), Some((1, 1..3)));
        // A long segment between two short ones keeps them apart.
        let apart = [
            (0., 0.5),
            (0.5, 0.02),
            (0.52, 0.12),
            (0.64, 0.02),
            (0.66, 0.34),
        ];
        assert_eq!(aim(&apart, 200., 102.), Some((1, 1..2)));
        assert_eq!(aim(&apart, 200., 116.), Some((2, 2..3)));
        assert_eq!(aim(&apart, 200., 130.), Some((3, 3..4)));
    }
}
