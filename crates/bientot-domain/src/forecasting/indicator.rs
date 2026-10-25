use jiff::{SignedDuration, Timestamp};

use super::{InvalidForecastInput, check_work_and_target};
use crate::summaries::IndicatorState;

/// Blue once the target is reached; otherwise green, orange or red as the projected departure
/// nears and passes the preferred latest departure.
pub fn indicator_state(
    credited_work: SignedDuration,
    effective_target: SignedDuration,
    projected_departure: Option<Timestamp>,
    preferred_latest_departure: Timestamp,
    orange_warning_margin: SignedDuration,
) -> Result<IndicatorState, InvalidForecastInput> {
    check_work_and_target(credited_work, effective_target)?;
    if orange_warning_margin.is_negative() {
        return Err(InvalidForecastInput::NegativeWarningMargin);
    }
    if credited_work >= effective_target {
        return Ok(IndicatorState::Blue);
    }
    let projected_departure =
        projected_departure.ok_or(InvalidForecastInput::MissingDepartureProjection)?;
    Ok(
        if projected_departure <= preferred_latest_departure - orange_warning_margin {
            IndicatorState::Green
        } else if projected_departure <= preferred_latest_departure {
            IndicatorState::Orange
        } else {
            IndicatorState::Red
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const EFFECTIVE_TARGET: SignedDuration = SignedDuration::from_hours(7);
    const WARNING_MARGIN: SignedDuration = SignedDuration::from_mins(15);
    const TICK: SignedDuration = SignedDuration::from_nanos(100);

    fn latest_departure() -> Timestamp {
        "2026-08-14T18:30:00Z".parse().unwrap()
    }

    #[test]
    fn forecast_colours_respect_exact_threshold_boundaries() {
        for (minutes_after_warning, ticks, expected) in [
            (-1, 0, IndicatorState::Green),
            (0, 0, IndicatorState::Green),
            (0, 1, IndicatorState::Orange),
            (15, 0, IndicatorState::Orange),
            (15, 1, IndicatorState::Red),
        ] {
            let projection = latest_departure() - WARNING_MARGIN
                + SignedDuration::from_mins(minutes_after_warning)
                + TICK * ticks;

            let state = indicator_state(
                EFFECTIVE_TARGET - TICK,
                EFFECTIVE_TARGET,
                Some(projection),
                latest_departure(),
                WARNING_MARGIN,
            );

            assert_eq!(state, Ok(expected), "{minutes_after_warning}m+{ticks}");
        }
    }

    #[test]
    fn blue_overrides_forecast_at_and_beyond_the_effective_target() {
        for ticks in [0, 1] {
            let state = indicator_state(
                EFFECTIVE_TARGET + TICK * ticks,
                EFFECTIVE_TARGET,
                Some(latest_departure() + SignedDuration::from_hours(2)),
                latest_departure(),
                WARNING_MARGIN,
            );

            assert_eq!(state, Ok(IndicatorState::Blue));
        }
    }

    #[test]
    fn blue_does_not_require_an_unused_departure_projection() {
        let state = indicator_state(
            EFFECTIVE_TARGET,
            EFFECTIVE_TARGET,
            None,
            latest_departure(),
            WARNING_MARGIN,
        );

        assert_eq!(state, Ok(IndicatorState::Blue));
    }

    #[test]
    fn incomplete_day_requires_a_projection() {
        let state = indicator_state(
            EFFECTIVE_TARGET - TICK,
            EFFECTIVE_TARGET,
            None,
            latest_departure(),
            WARNING_MARGIN,
        );

        assert_eq!(state, Err(InvalidForecastInput::MissingDepartureProjection));
    }
}
