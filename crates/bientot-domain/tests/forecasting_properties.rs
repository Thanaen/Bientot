//! Properties of departure projection, overtime and the indicator colour.

use bientot_domain::forecasting::{indicator_state, overtime, project_departure};
use bientot_domain::intervals::UtcInterval;
use bientot_domain::summaries::IndicatorState;
use jiff::{SignedDuration, Timestamp};
use proptest::prelude::*;

fn minute(minute: i64) -> Timestamp {
    "2026-08-24T00:00:00Z".parse::<Timestamp>().unwrap() + SignedDuration::from_mins(minute)
}

fn minutes(minutes: i64) -> SignedDuration {
    SignedDuration::from_mins(minutes)
}

fn lunch(start: i64, length: i64) -> UtcInterval {
    UtcInterval::new(minute(start), minute(start + length)).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn more_credited_work_never_makes_the_departure_later(
        now in 360i64..1260,
        credited in 0i64..600,
        extra in 0i64..600,
        target in 1i64..600,
        (lunch_start, lunch_length) in (600i64..840, 1i64..180),
    ) {
        let lunch = lunch(lunch_start, lunch_length);
        let project = |work| project_departure(minute(now), minutes(work), minutes(target), lunch).unwrap();

        let (less, more) = (project(credited), project(credited + extra));

        prop_assert_eq!(less.is_none(), credited >= target);
        match (less, more) {
            (Some(less), Some(more)) => prop_assert!(more.projected_departure <= less.projected_departure),
            (Some(_), None) | (None, None) => {}
            (None, Some(_)) => prop_assert!(false, "a complete day became incomplete"),
        }
    }

    #[test]
    fn the_departure_reserves_at_most_the_lunch_break(
        now in 360i64..1260,
        credited in 0i64..600,
        target in 1i64..600,
        (lunch_start, lunch_length) in (600i64..840, 1i64..180),
    ) {
        let lunch = lunch(lunch_start, lunch_length);
        if let Some(projection) = project_departure(minute(now), minutes(credited), minutes(target), lunch).unwrap() {
            let earliest = minute(now) + minutes(target - credited);
            prop_assert!(projection.projected_departure >= earliest);
            prop_assert!(projection.projected_departure <= earliest + lunch.duration());
            prop_assert_eq!(projection.lunch_over, minute(now) >= lunch.end());
            prop_assert!(!lunch.contains(projection.projected_departure) || projection.projected_departure == lunch.start());
        }
    }

    #[test]
    fn overtime_is_never_negative_and_complements_the_remaining_work(
        credited in 0i64..2000,
        target in 1i64..1440,
    ) {
        let overtime = overtime(minutes(credited), minutes(target)).unwrap();
        let remaining = minutes(target - credited).max(SignedDuration::ZERO);

        prop_assert!(!overtime.is_negative());
        prop_assert_eq!(overtime - remaining, minutes(credited - target));
    }

    #[test]
    fn the_colour_is_blue_exactly_at_the_target_and_worsens_with_a_later_departure(
        credited in 0i64..600,
        target in 1i64..600,
        departure in 900i64..1300,
        delay in 0i64..300,
        margin in 0i64..60,
    ) {
        let latest = minute(1110);
        let state = |at| indicator_state(minutes(credited), minutes(target), Some(minute(at)), latest, minutes(margin)).unwrap();
        let rank = |state| match state {
            IndicatorState::Green => 0,
            IndicatorState::Orange => 1,
            IndicatorState::Red => 2,
            IndicatorState::Blue => 3,
        };

        let (earlier, later) = (state(departure), state(departure + delay));

        prop_assert_eq!(earlier == IndicatorState::Blue, credited >= target);
        prop_assert!(rank(earlier) <= rank(later));
    }
}
