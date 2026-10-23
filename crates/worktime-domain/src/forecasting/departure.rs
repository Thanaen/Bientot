use jiff::{SignedDuration, Timestamp};

use super::{InvalidForecastInput, check_work_and_target};
use crate::intervals::UtcInterval;

/// Forecast for an incomplete workday.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepartureProjection {
    pub projected_departure: Timestamp,
    /// The lunch window has ended: the departure no longer reserves any of it.
    pub lunch_over: bool,
}

/// When the remaining work ends if it accrues one for one from `now`, keeping the rest of lunch
/// free. `None` once the target is reached.
pub fn project_departure(
    now: Timestamp,
    credited_work: SignedDuration,
    effective_target: SignedDuration,
    lunch: UtcInterval,
) -> Result<Option<DepartureProjection>, InvalidForecastInput> {
    check_work_and_target(credited_work, effective_target)?;
    if credited_work >= effective_target {
        return Ok(None);
    }
    let mut projected_departure = now + (effective_target - credited_work);
    if now < lunch.start() && projected_departure > lunch.start() {
        projected_departure += lunch.duration();
    } else if lunch.contains(now) {
        projected_departure += lunch.end().duration_since(now);
    }
    Ok(Some(DepartureProjection {
        projected_departure,
        lunch_over: now >= lunch.end(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET: SignedDuration = SignedDuration::from_hours(7);

    fn at(hour: i8, minute: i8) -> Timestamp {
        jiff::civil::date(2026, 8, 24)
            .at(hour, minute, 0, 0)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .unwrap()
            .timestamp()
    }

    fn lunch() -> UtcInterval {
        UtcInterval::new(at(12, 0), at(14, 0)).unwrap()
    }

    fn project(now: Timestamp, credited_minutes: i64) -> Option<DepartureProjection> {
        project_departure(
            now,
            SignedDuration::from_mins(credited_minutes),
            TARGET,
            lunch(),
        )
        .unwrap()
    }

    fn projection(projected_departure: Timestamp, lunch_over: bool) -> Option<DepartureProjection> {
        Some(DepartureProjection {
            projected_departure,
            lunch_over,
        })
    }

    #[test]
    fn before_lunch_reserves_the_whole_lunch_window_when_work_crosses_it() {
        assert_eq!(project(at(9, 0), 0), projection(at(18, 0), false));
    }

    #[test]
    fn before_lunch_does_not_reserve_lunch_when_work_finishes_at_its_start() {
        assert_eq!(project(at(11, 30), 390), projection(at(12, 0), false));
    }

    #[test]
    fn at_lunch_start_reserves_the_whole_lunch_window() {
        assert_eq!(project(at(12, 0), 180), projection(at(18, 0), false));
    }

    #[test]
    fn during_lunch_reserves_only_the_still_future_lunch_time() {
        assert_eq!(project(at(13, 0), 210), projection(at(17, 30), false));
    }

    #[test]
    fn at_lunch_end_uses_actual_credit_without_further_reservation_and_becomes_visible() {
        assert_eq!(project(at(14, 0), 240), projection(at(17, 0), true));
    }

    #[test]
    fn after_lunch_assumes_remaining_elapsed_minutes_accrue_one_for_one() {
        assert_eq!(project(at(15, 20), 315), projection(at(17, 5), true));
    }

    #[test]
    fn complete_day_has_no_departure_projection() {
        for credited in [420, 450] {
            assert_eq!(project(at(16, 0), credited), None);
        }
    }

    /// The .NET test also rejects a lunch ending before it starts; [`UtcInterval`] cannot hold one.
    #[test]
    fn invalid_inputs_are_rejected() {
        assert_eq!(
            project_departure(at(9, 0), SignedDuration::from_mins(-1), TARGET, lunch()),
            Err(InvalidForecastInput::NegativeCreditedWork)
        );
        assert_eq!(
            project_departure(
                at(9, 0),
                SignedDuration::ZERO,
                SignedDuration::ZERO,
                lunch()
            ),
            Err(InvalidForecastInput::NonPositiveTarget)
        );
        assert!(UtcInterval::new(at(14, 0), at(12, 0)).is_err());
    }
}
