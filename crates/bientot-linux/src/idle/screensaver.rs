//! `org.freedesktop.ScreenSaver.GetSessionIdleTime`, whose unit differs between the desktops
//! that implement it: milliseconds on some, seconds on others. The backend is used only once
//! [`UNIT_CHECK_READS`] reads [`UNIT_CHECK_STEP`] apart tell which.

#[cfg(target_os = "linux")]
mod native;

#[cfg(target_os = "linux")]
pub(super) use native::open;

use std::time::Duration;

/// How many reads the unit check takes.
pub const UNIT_CHECK_READS: usize = 11;

/// How long the unit check waits between two reads: the reads span a second.
pub const UNIT_CHECK_STEP: Duration = Duration::from_millis(100);

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

/// One answer of the unit check, and the monotonic instant it came back at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Read {
    pub at: Duration,
    pub value: u32,
}

/// How much a millisecond count may stray from the time between the first and the last read:
/// the calls' own latency.
const MILLISECOND_TOLERANCE: Duration = Duration::from_millis(250);

/// The unit of `reads`, in the order they were taken, or `None` when they do not tell: input
/// came between them, or the answers do not move with the time.
///
/// Input resets the count, so a count that goes down anywhere tells nothing, however its first
/// and last answers compare.
pub fn unit(reads: &[Read]) -> Option<Unit> {
    let steps = reads
        .windows(2)
        .map(|pair| pair[1].value.checked_sub(pair[0].value))
        .collect::<Option<Vec<u32>>>()?;
    let (first, last) = (reads.first()?, reads.last()?);
    let grew = last.value - first.value;
    let elapsed = last.at.saturating_sub(first.at);
    if Duration::from_millis(grew.into()).abs_diff(elapsed) <= MILLISECOND_TOLERANCE {
        return Some(Unit::Milliseconds);
    }
    // Whole seconds: the count moves by the seconds that elapsed, give or take the one the
    // first and last reads fall either side of, and never by more than one between two reads.
    (grew >= 1
        && (f64::from(grew) - elapsed.as_secs_f64()).abs() < 1.0
        && steps.iter().all(|&step| step <= 1))
    .then_some(Unit::Seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `values` read [`UNIT_CHECK_STEP`] apart, the last one `late` after its turn.
    fn reads(values: &[u32], late: Duration) -> Vec<Read> {
        let mut reads: Vec<Read> = (0..)
            .zip(values)
            .map(|(index, &value)| Read {
                at: UNIT_CHECK_STEP * index,
                value,
            })
            .collect();
        if let Some(last) = reads.last_mut() {
            last.at += late;
        }
        reads
    }

    fn on_time(values: &[u32]) -> Vec<Read> {
        reads(values, Duration::ZERO)
    }

    #[test]
    fn a_count_that_grows_with_the_time_in_milliseconds_is_in_milliseconds() {
        let steady = [
            4_200, 4_301, 4_400, 4_502, 4_600, 4_700, 4_801, 4_900, 5_000, 5_101, 5_210,
        ];
        assert_eq!(unit(&on_time(&steady)), Some(Unit::Milliseconds));
        let slow_last_call = [0, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1_190];
        assert_eq!(
            unit(&reads(&slow_last_call, Duration::from_millis(200))),
            Some(Unit::Milliseconds)
        );
    }

    #[test]
    fn a_count_that_grows_by_one_per_second_is_in_seconds() {
        assert_eq!(
            unit(&on_time(&[42, 42, 42, 42, 43, 43, 43, 43, 43, 43, 43])),
            Some(Unit::Seconds)
        );
        assert_eq!(
            unit(&reads(
                &[42, 43, 43, 43, 43, 43, 43, 43, 43, 43, 44],
                Duration::from_millis(30)
            )),
            Some(Unit::Seconds),
            "the reads fall either side of two second boundaries"
        );
    }

    #[test]
    fn a_millisecond_count_that_input_resets_during_the_check_tells_nothing() {
        // The pointer moves throughout: each answer is the few milliseconds since its last
        // motion, and the first and last answers differ by one as a count in seconds would.
        assert_eq!(unit(&on_time(&[5, 12, 3, 9, 14, 2, 7, 11, 4, 8, 6])), None);
        // One key press during the check.
        assert_eq!(
            unit(&on_time(&[
                5_000, 5_100, 5_200, 5_300, 40, 140, 240, 340, 440, 540, 640
            ])),
            None
        );
    }

    #[test]
    fn a_count_that_stands_still_or_moves_too_fast_tells_nothing() {
        assert_eq!(unit(&on_time(&[0; 11])), None, "the user typed throughout");
        assert_eq!(unit(&on_time(&[7; 11])), None);
        assert_eq!(
            unit(&on_time(&[
                100, 150, 200, 250, 300, 350, 400, 450, 500, 550, 600
            ])),
            None,
            "half a second in a second"
        );
        assert_eq!(
            unit(&on_time(&[1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4])),
            None,
            "three seconds in a second"
        );
        assert_eq!(
            unit(&reads(
                &[42, 42, 42, 42, 42, 44, 44, 44, 44, 44, 44],
                Duration::from_millis(30)
            )),
            None,
            "two seconds between two reads"
        );
    }

    #[test]
    fn the_unit_converts_the_answer() {
        assert_eq!(Unit::Milliseconds.idle(1_500), Duration::from_millis(1_500));
        assert_eq!(Unit::Seconds.idle(90), Duration::from_secs(90));
    }
}
