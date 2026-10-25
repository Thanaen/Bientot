//! Half-open UTC intervals and normalized sets of them.

use jiff::{SignedDuration, Timestamp};

/// A non-empty half-open interval `[start, end)` in UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcInterval {
    start: Timestamp,
    end: Timestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmptyInterval;

impl std::fmt::Display for EmptyInterval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an interval must have a positive duration")
    }
}

impl std::error::Error for EmptyInterval {}

impl UtcInterval {
    pub fn new(start: Timestamp, end: Timestamp) -> Result<Self, EmptyInterval> {
        if end <= start {
            return Err(EmptyInterval);
        }
        Ok(Self { start, end })
    }

    pub fn start(self) -> Timestamp {
        self.start
    }

    pub fn end(self) -> Timestamp {
        self.end
    }

    pub fn duration(self) -> SignedDuration {
        self.end.duration_since(self.start)
    }

    pub fn contains(self, instant: Timestamp) -> bool {
        self.start <= instant && instant < self.end
    }

    pub fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    pub fn touches_or_overlaps(self, other: Self) -> bool {
        self.start <= other.end && other.start <= self.end
    }

    /// The smallest interval covering both, or `None` when a gap separates them.
    pub fn merge(self, other: Self) -> Option<Self> {
        self.touches_or_overlaps(other).then(|| Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        })
    }

    pub fn intersect(self, other: Self) -> Option<Self> {
        Self::new(self.start.max(other.start), self.end.min(other.end)).ok()
    }
}

/// Sorted intervals with no overlap and no contact: touching inputs are merged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IntervalSet {
    intervals: Vec<UtcInterval>,
}

impl IntervalSet {
    pub fn new(intervals: impl IntoIterator<Item = UtcInterval>) -> Self {
        let mut sorted: Vec<_> = intervals.into_iter().collect();
        sorted.sort_unstable();
        let mut merged: Vec<UtcInterval> = Vec::with_capacity(sorted.len());
        for interval in sorted {
            if let Some(last) = merged.last_mut()
                && let Some(union) = last.merge(interval)
            {
                *last = union;
            } else {
                merged.push(interval);
            }
        }
        Self { intervals: merged }
    }

    pub fn len(&self) -> usize {
        self.intervals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.intervals.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, UtcInterval> {
        self.intervals.iter()
    }

    pub fn as_slice(&self) -> &[UtcInterval] {
        &self.intervals
    }

    pub fn covered_duration(&self) -> SignedDuration {
        self.intervals
            .iter()
            .fold(SignedDuration::ZERO, |total, interval| {
                total + interval.duration()
            })
    }

    pub fn add(&self, interval: UtcInterval) -> Self {
        Self::new(self.intervals.iter().copied().chain([interval]))
    }

    pub fn union(&self, other: &Self) -> Self {
        Self::new(self.intervals.iter().chain(&other.intervals).copied())
    }

    pub fn intersect(&self, bounds: UtcInterval) -> Self {
        Self::new(
            self.intervals
                .iter()
                .filter_map(|interval| interval.intersect(bounds)),
        )
    }

    pub fn subtract(&self, excluded: UtcInterval) -> Self {
        Self::new(self.intervals.iter().flat_map(|&interval| {
            let before = UtcInterval::new(interval.start, interval.end.min(excluded.start));
            let after = UtcInterval::new(interval.start.max(excluded.end), interval.end);
            [before.ok(), after.ok()].into_iter().flatten()
        }))
    }
}

impl<'a> IntoIterator for &'a IntervalSet {
    type Item = &'a UtcInterval;
    type IntoIter = std::slice::Iter<'a, UtcInterval>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl FromIterator<UtcInterval> for IntervalSet {
    fn from_iter<I: IntoIterator<Item = UtcInterval>>(iter: I) -> Self {
        Self::new(iter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::ToSpan;

    fn at(hour: i8, minute: i8) -> Timestamp {
        jiff::civil::date(2026, 8, 3)
            .at(hour, minute, 0, 0)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .unwrap()
            .timestamp()
    }

    fn interval(start: Timestamp, end: Timestamp) -> UtcInterval {
        UtcInterval::new(start, end).unwrap()
    }

    #[test]
    fn normalization_merges_overlapping_and_adjacent_intervals() {
        let set = IntervalSet::new([
            interval(at(8, 0), at(9, 0)),
            interval(at(8, 30), at(9, 30)),
            interval(at(9, 30), at(10, 0)),
        ]);

        assert_eq!(set.as_slice(), [interval(at(8, 0), at(10, 0))]);
        assert_eq!(set.covered_duration(), SignedDuration::from_hours(2));
    }

    #[test]
    fn subtract_splits_coverage_without_double_counting() {
        let set = IntervalSet::new([interval(at(8, 0), at(12, 0))]);

        let result = set.subtract(interval(at(9, 0), at(11, 0)));

        assert_eq!(
            result.as_slice(),
            [interval(at(8, 0), at(9, 0)), interval(at(11, 0), at(12, 0))]
        );
        assert_eq!(result.covered_duration(), SignedDuration::from_hours(2));
    }

    #[test]
    fn interval_rejects_zero_or_negative_duration() {
        let instant = at(8, 0);

        assert_eq!(UtcInterval::new(instant, instant), Err(EmptyInterval));
        assert_eq!(
            UtcInterval::new(instant, instant - 1.second()),
            Err(EmptyInterval)
        );
    }

    #[test]
    fn contains_includes_start_and_excludes_end() {
        let morning = interval(at(8, 0), at(9, 0));

        assert!(morning.contains(at(8, 0)));
        assert!(!morning.contains(at(9, 0)));
    }

    #[test]
    fn merge_refuses_disjoint_intervals() {
        assert_eq!(
            interval(at(8, 0), at(9, 0)).merge(interval(at(9, 1), at(10, 0))),
            None
        );
    }
}
