use jiff::SignedDuration;

use super::{InvalidForecastInput, check_work_and_target};

/// Credited work beyond the effective target.
pub fn overtime(
    credited_work: SignedDuration,
    effective_target: SignedDuration,
) -> Result<SignedDuration, InvalidForecastInput> {
    check_work_and_target(credited_work, effective_target)?;
    Ok(excess(credited_work, effective_target))
}

pub(crate) fn excess(value: SignedDuration, limit: SignedDuration) -> SignedDuration {
    (value - limit).max(SignedDuration::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET: SignedDuration = SignedDuration::from_hours(7);

    fn minutes(minutes: i64) -> SignedDuration {
        SignedDuration::from_mins(minutes)
    }

    #[test]
    fn before_or_at_target_overtime_is_zero() {
        for credited in [0, 390, 420] {
            assert_eq!(
                overtime(minutes(credited), TARGET),
                Ok(SignedDuration::ZERO),
                "{credited}"
            );
        }
    }

    #[test]
    fn after_target_overtime_grows_with_newly_credited_work() {
        for (credited, expected) in [(421, 1), (450, 30), (480, 60)] {
            assert_eq!(overtime(minutes(credited), TARGET), Ok(minutes(expected)));
        }
    }

    #[test]
    fn applied_carry_lowers_the_point_where_new_overtime_starts() {
        let effective_target = minutes(390);

        assert_eq!(
            overtime(minutes(390), effective_target),
            Ok(SignedDuration::ZERO)
        );
        assert_eq!(overtime(minutes(420), effective_target), Ok(minutes(30)));
    }

    #[test]
    fn invalid_inputs_are_rejected() {
        assert_eq!(
            overtime(minutes(-1), TARGET),
            Err(InvalidForecastInput::NegativeCreditedWork)
        );
        assert_eq!(
            overtime(SignedDuration::ZERO, SignedDuration::ZERO),
            Err(InvalidForecastInput::NonPositiveTarget)
        );
    }
}
