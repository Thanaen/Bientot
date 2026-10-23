//! Properties of the source rules: gap completion covers the bounds exactly, and the idle tracker
//! produces a gapless, monotonic span history whatever the sampling and input pattern.

use jiff::{SignedDuration, Timestamp};
use proptest::prelude::*;
use worktime_domain::accounting::{
    ActivityCoverage, ActivitySegment, ActivityState, AutomaticOrigin,
};
use worktime_domain::intervals::{IntervalSet, UtcInterval};
use worktime_domain::sources::{
    IdleObservation, IdleStateTracker, LastInputReading, complete_unknown_gaps,
};

fn origin() -> Timestamp {
    "2026-08-03T09:00:00Z".parse().unwrap()
}

fn minute(minute: i64) -> Timestamp {
    origin() + SignedDuration::from_mins(minute)
}

fn interval(first: i64, second: i64) -> UtcInterval {
    UtcInterval::new(minute(first.min(second)), minute(first.max(second) + 1)).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn completed_coverage_spans_the_bounds_and_gaps_avoid_evidence(
        bounds in (0i64..60, 0i64..60),
        evidence in prop::collection::vec((-10i64..70, -10i64..70, 0usize..3), 0..8),
    ) {
        let bounds = interval(bounds.0, bounds.1);
        let source = AutomaticOrigin::new("source").unwrap();
        let gap_origin = AutomaticOrigin::new("gap").unwrap();
        let states = [ActivityState::Active, ActivityState::Afk, ActivityState::Unknown];
        let coverage = ActivityCoverage::new(
            interval(-10, 70),
            evidence.iter().map(|&(a, b, state)| {
                ActivitySegment::new(interval(a, b), states[state], source.clone())
            }),
        );

        let completed = complete_unknown_gaps(bounds, &coverage, &gap_origin);

        let all: IntervalSet = completed.segments().iter().map(|segment| segment.interval).collect();
        prop_assert_eq!(all.as_slice(), [bounds]);
        let evidence: IntervalSet = completed
            .segments()
            .iter()
            .filter(|segment| segment.origin == source)
            .map(|segment| segment.interval)
            .collect();
        let gaps: Vec<_> = completed
            .segments()
            .iter()
            .filter(|segment| segment.origin == gap_origin)
            .collect();
        for gap in &gaps {
            prop_assert_eq!(gap.state, ActivityState::Unknown);
            prop_assert!(evidence.intersect(gap.interval).is_empty());
        }
    }

    #[test]
    fn idle_spans_chain_without_gaps_and_never_settle_past_the_sample(
        steps in prop::collection::vec((1i64..12_000, prop::option::of(0.0f64..1.0)), 1..200),
        tick_at_origin in any::<u32>(),
        threshold_secs in 1i64..30,
    ) {
        let threshold = SignedDuration::from_secs(threshold_secs);
        let mut tracker = IdleStateTracker::default();
        let mut elapsed_ms = 0i64;
        let mut last_input_tick = tick_at_origin;
        let tick = |ms: i64| tick_at_origin.wrapping_add(ms as u32);

        for (step_ms, input) in steps {
            let previous_ms = elapsed_ms;
            elapsed_ms += step_ms;
            if let Some(fraction) = input {
                last_input_tick = tick(previous_ms + ((step_ms as f64) * fraction) as i64);
            }
            let at = origin() + SignedDuration::from_millis(elapsed_ms);
            let previous = tracker.open();
            let reading = LastInputReading { tick_count: tick(elapsed_ms), last_input_tick };

            let observation = tracker.observe(at, reading, threshold);

            let open = observation.open();
            prop_assert!(open.start() <= open.settled_through());
            prop_assert!(open.settled_through() <= at);
            prop_assert_eq!(tracker.open(), Some(open));
            match observation {
                IdleObservation::Continued { open } => {
                    let previous = previous.unwrap();
                    prop_assert_eq!((open.state(), open.start()), (previous.state(), previous.start()));
                    prop_assert!(open.settled_through() >= previous.settled_through());
                }
                IdleObservation::Transitioned { closed, open } => {
                    let previous = previous.unwrap();
                    prop_assert_eq!((closed.state(), closed.start()), (previous.state(), previous.start()));
                    prop_assert!(closed.settled_through() >= previous.settled_through());
                    prop_assert_eq!(closed.settled_through(), open.start());
                    prop_assert_ne!(closed.state(), open.state());
                }
                IdleObservation::Started { open, abandoned } => {
                    prop_assert_eq!(abandoned, previous);
                    prop_assert_eq!(open.start(), at);
                }
            }
        }
    }
}
