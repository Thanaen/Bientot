//! Interval set operations against a boolean grid oracle, as in the .NET
//! `IntervalPropertyTests`: one nanosecond per cell over 256 cells.

use bientot_domain::intervals::{IntervalSet, UtcInterval};
use jiff::{SignedDuration, Timestamp};
use proptest::prelude::*;

const GRID: usize = 256;

fn origin() -> Timestamp {
    "2026-08-03T00:00:00Z".parse().unwrap()
}

fn range(first: u8, second: u8) -> (usize, usize) {
    (
        usize::from(first.min(second)),
        usize::from(first.max(second)) + 1,
    )
}

fn interval((start, end): (usize, usize)) -> UtcInterval {
    let at = |cell: usize| origin() + SignedDuration::from_nanos(cell as i64);
    UtcInterval::new(at(start), at(end)).unwrap()
}

fn offset(instant: Timestamp) -> usize {
    instant.duration_since(origin()).as_nanos() as usize
}

fn intervals(endpoints: &[(u8, u8)]) -> Vec<UtcInterval> {
    endpoints
        .iter()
        .map(|&(a, b)| interval(range(a, b)))
        .collect()
}

fn occupied(endpoints: &[(u8, u8)]) -> [bool; GRID] {
    let mut cells = [false; GRID];
    for &(a, b) in endpoints {
        let (start, end) = range(a, b);
        cells[start..end].fill(true);
    }
    cells
}

fn count(cells: &[bool; GRID]) -> usize {
    cells.iter().filter(|&&cell| cell).count()
}

fn assert_matches(set: &IntervalSet, expected: &[bool; GRID]) {
    let mut cells = [false; GRID];
    let mut previous_end = None;
    for interval in set {
        let (start, end) = (offset(interval.start()), offset(interval.end()));
        assert!(start < end && end <= GRID);
        if let Some(previous) = previous_end {
            assert!(previous < start, "normalized intervals must be separated");
        }
        for cell in &mut cells[start..end] {
            assert!(!*cell, "normalized intervals must not cover a cell twice");
            *cell = true;
        }
        previous_end = Some(end);
    }
    assert_eq!(&cells, expected);
    assert_eq!(set.covered_duration().as_nanos(), count(expected) as i128);
}

fn endpoints() -> impl Strategy<Value = Vec<(u8, u8)>> {
    prop::collection::vec(any::<(u8, u8)>(), 0..32)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn normalization_matches_occupied_cells(endpoints in endpoints()) {
        let set = IntervalSet::new(intervals(&endpoints));

        assert_matches(&set, &occupied(&endpoints));
        let reversed: IntervalSet = intervals(&endpoints).into_iter().rev().collect();
        prop_assert_eq!(&reversed, &set);
    }

    #[test]
    fn union_is_boolean_or_commutative_and_idempotent(
        left in endpoints(),
        right in endpoints(),
    ) {
        let (a, b) = (IntervalSet::new(intervals(&left)), IntervalSet::new(intervals(&right)));
        let mut expected = occupied(&left);
        for (cell, other) in expected.iter_mut().zip(occupied(&right)) {
            *cell |= other;
        }

        let union = a.union(&b);

        assert_matches(&union, &expected);
        prop_assert_eq!(&b.union(&a), &union);
        prop_assert_eq!(&a.union(&a), &a);
    }

    #[test]
    fn intersection_is_boolean_and_within_bounds(
        endpoints in endpoints(),
        bound in any::<(u8, u8)>(),
    ) {
        let source = IntervalSet::new(intervals(&endpoints));
        let (start, end) = range(bound.0, bound.1);
        let mut expected = occupied(&endpoints);
        for (cell, value) in expected.iter_mut().enumerate() {
            *value &= (start..end).contains(&cell);
        }

        let result = source.intersect(interval((start, end)));

        assert_matches(&result, &expected);
        prop_assert!(result.covered_duration() <= source.covered_duration());
    }

    #[test]
    fn subtraction_is_difference_and_conserves_duration(
        endpoints in endpoints(),
        excluded in any::<(u8, u8)>(),
    ) {
        let source = IntervalSet::new(intervals(&endpoints));
        let (start, end) = range(excluded.0, excluded.1);
        let before = occupied(&endpoints);
        let mut remaining = before;
        remaining[start..end].fill(false);
        let removed = count(&before) - count(&remaining);

        let result = source.subtract(interval((start, end)));

        assert_matches(&result, &remaining);
        prop_assert_eq!(count(&before), removed + count(&remaining));
    }
}
