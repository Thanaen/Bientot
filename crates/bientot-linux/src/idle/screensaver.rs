//! `org.freedesktop.ScreenSaver.GetSessionIdleTime`, whose unit differs between the desktops
//! that implement it: milliseconds on some, seconds on others. The backend is used only once two
//! reads [`UNIT_CHECK_GAP`] apart tell which.

#[cfg(target_os = "linux")]
mod native;

#[cfg(target_os = "linux")]
pub(super) use native::open;

use std::time::Duration;

/// How far apart the two reads of the unit check are taken.
pub const UNIT_CHECK_GAP: Duration = Duration::from_secs(1);

/// The unit `GetSessionIdleTime` answers in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Milliseconds,
    Seconds,
}

impl Unit {
    pub fn idle(self, value: u32) -> Duration {
        match self {
            Self::Milliseconds => Duration::from_millis(value.into()),
            Self::Seconds => Duration::from_secs(value.into()),
        }
    }
}

/// How much a millisecond count may stray from the time between the reads: the two calls'
/// own latency.
const MILLISECOND_TOLERANCE: Duration = Duration::from_millis(250);

/// The unit of two answers read `elapsed` apart without input in between, or `None` when they
/// do not tell: input came between them, or the answers do not move with the time.
pub fn unit(first: u32, second: u32, elapsed: Duration) -> Option<Unit> {
    let grew = second.checked_sub(first)?;
    if Duration::from_millis(grew.into()).abs_diff(elapsed) <= MILLISECOND_TOLERANCE {
        return Some(Unit::Milliseconds);
    }
    // Whole seconds: the count moves by the seconds that elapsed, give or take the one the two
    // reads fall either side of.
    (grew >= 1 && (f64::from(grew) - elapsed.as_secs_f64()).abs() < 1.0).then_some(Unit::Seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAP: Duration = UNIT_CHECK_GAP;

    #[test]
    fn a_count_that_grows_by_the_gap_in_milliseconds_is_in_milliseconds() {
        assert_eq!(unit(4_200, 5_210, GAP), Some(Unit::Milliseconds));
        assert_eq!(
            unit(0, 1_190, GAP + Duration::from_millis(200)),
            Some(Unit::Milliseconds)
        );
    }

    #[test]
    fn a_count_that_grows_by_one_per_second_is_in_seconds() {
        assert_eq!(unit(42, 43, GAP), Some(Unit::Seconds));
        assert_eq!(
            unit(42, 44, GAP + Duration::from_millis(30)),
            Some(Unit::Seconds)
        );
    }

    #[test]
    fn input_between_the_reads_or_a_count_that_stands_still_tells_nothing() {
        assert_eq!(unit(5_000, 300, GAP), None, "input reset the count");
        assert_eq!(unit(0, 0, GAP), None, "the user typed throughout");
        assert_eq!(unit(7, 7, GAP), None);
        assert_eq!(unit(100, 600, GAP), None, "half a second in a second");
        assert_eq!(unit(1, 4, GAP), None, "three seconds in a second");
    }

    #[test]
    fn the_unit_converts_the_answer() {
        assert_eq!(Unit::Milliseconds.idle(1_500), Duration::from_millis(1_500));
        assert_eq!(Unit::Seconds.idle(90), Duration::from_secs(90));
    }
}
