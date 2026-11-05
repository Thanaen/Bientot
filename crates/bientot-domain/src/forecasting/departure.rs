use jiff::{SignedDuration, Timestamp};

use super::{InvalidForecastInput, check_work_and_target};
use crate::accounting::{AccountingLabel, AccountingTimeline, SegmentKind};
use crate::intervals::UtcInterval;

/// Time away inside the lunch window from which a return to work ends the lunch.
const MINIMUM_LUNCH_BREAK: SignedDuration = SignedDuration::from_mins(15);
/// Work long enough after the break to tell a return from a glance at the PC.
const MINIMUM_RETURN: SignedDuration = SignedDuration::from_mins(5);

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

/// The lunch window as the departure should reserve it: ended where the user came back to work
/// after at least [`MINIMUM_LUNCH_BREAK`] away inside it and stayed [`MINIMUM_RETURN`], so that a
/// shortened lunch no longer holds the departure back (issue #135). A lock joined to lunch
/// counts in full, as it does in the accounting; unknown time does not count.
pub fn lunch_as_taken(lunch: UtcInterval, timeline: &AccountingTimeline) -> UtcInterval {
    let mut away = SignedDuration::ZERO;
    let mut back: Option<(Timestamp, SignedDuration)> = None;
    for segment in timeline.segments() {
        let interval = segment.interval();
        // Unknown time says neither that the user was away nor that they worked.
        if *segment.kind() == SegmentKind::Unknown {
            continue;
        }
        if *segment.kind() == SegmentKind::Excluded {
            back = None;
            away += if segment.label() == AccountingLabel::LunchLock {
                interval.duration()
            } else {
                interval
                    .intersect(lunch)
                    .map_or(SignedDuration::ZERO, UtcInterval::duration)
            };
            continue;
        }
        let (returned, worked) = back.get_or_insert((interval.start(), SignedDuration::ZERO));
        *worked += interval.duration();
        if *worked < MINIMUM_RETURN {
            continue;
        }
        // A return right at the start of the window, possible only after a corrected lock, keeps it.
        if away >= MINIMUM_LUNCH_BREAK
            && let Ok(taken) = UtcInterval::new(lunch.start(), *returned)
            && *returned < lunch.end()
        {
            return taken;
        }
        // Back at work from shorter breaks: the next break starts the count again.
        away = SignedDuration::ZERO;
    }
    lunch
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::{AccountedSegment, AutomaticOrigin};
    use crate::time::{LocalWorkday, WorkdayWindow};

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

    #[derive(Clone, Copy)]
    enum Piece {
        Work,
        Away,
        LockedAtLunch,
        Unknown,
    }

    /// A timeline from 08:00 made of `pieces` that each end at the given time.
    fn timeline(pieces: &[(Piece, Timestamp)]) -> AccountingTimeline {
        let origin = AutomaticOrigin::new("test").unwrap();
        let mut start = at(8, 0);
        let segments = pieces
            .iter()
            .map(|&(piece, end)| {
                let interval = UtcInterval::new(start, end).unwrap();
                start = end;
                match piece {
                    Piece::Work => AccountedSegment::credited(
                        interval,
                        AccountingLabel::ActiveWork,
                        origin.clone(),
                    ),
                    Piece::Away => {
                        AccountedSegment::excluded(interval, AccountingLabel::Afk, origin.clone())
                    }
                    Piece::LockedAtLunch => AccountedSegment::excluded(
                        interval,
                        AccountingLabel::LunchLock,
                        origin.clone(),
                    ),
                    Piece::Unknown => AccountedSegment::unknown(interval, origin.clone()),
                }
            })
            .collect();
        let window = WorkdayWindow {
            workday: LocalWorkday::new(jiff::civil::date(2026, 8, 24), "UTC"),
            start: at(8, 0),
            end: at(23, 0),
        };
        AccountingTimeline::new(window, start, segments)
    }

    fn lunch_until(hour: i8, minute: i8) -> UtcInterval {
        UtcInterval::new(at(12, 0), at(hour, minute)).unwrap()
    }

    #[test]
    fn a_return_from_a_shortened_lunch_ends_the_lunch_there() {
        let back = timeline(&[
            (Piece::Work, at(12, 0)),
            (Piece::Away, at(12, 40)),
            (Piece::Work, at(12, 45)),
        ]);
        assert_eq!(lunch_as_taken(lunch(), &back), lunch_until(12, 40));
    }

    #[test]
    fn the_departure_stops_moving_once_back_from_a_shortened_lunch() {
        // Back at 12:40 with 4 h 05 credited: 2 h 55 left from 12:45, then from 12:46.
        let departures: Vec<_> = [(at(12, 45), 245), (at(12, 46), 246)]
            .into_iter()
            .map(|(now, credited)| {
                let back = timeline(&[
                    (Piece::Work, at(12, 0)),
                    (Piece::Away, at(12, 40)),
                    (Piece::Work, now),
                ]);
                project_departure(
                    now,
                    SignedDuration::from_mins(credited),
                    TARGET,
                    lunch_as_taken(lunch(), &back),
                )
                .unwrap()
            })
            .collect();
        assert_eq!(departures, vec![projection(at(15, 40), true); 2]);
    }

    #[test]
    fn the_first_minutes_back_still_reserve_the_window() {
        let glance = timeline(&[
            (Piece::Work, at(12, 0)),
            (Piece::Away, at(12, 40)),
            (Piece::Work, at(12, 44)),
        ]);
        assert_eq!(lunch_as_taken(lunch(), &glance), lunch());
    }

    #[test]
    fn a_glance_at_the_pc_during_lunch_keeps_the_window() {
        let glance = timeline(&[
            (Piece::Work, at(12, 0)),
            (Piece::Away, at(12, 40)),
            (Piece::Work, at(12, 43)),
            (Piece::Away, at(13, 0)),
        ]);
        assert_eq!(lunch_as_taken(lunch(), &glance), lunch());
    }

    #[test]
    fn a_short_break_inside_the_window_is_not_lunch() {
        let coffee = timeline(&[
            (Piece::Work, at(12, 5)),
            (Piece::Away, at(12, 19)),
            (Piece::Work, at(12, 40)),
        ]);
        assert_eq!(lunch_as_taken(lunch(), &coffee), lunch());
    }

    #[test]
    fn only_the_time_away_inside_the_window_counts_as_lunch() {
        let meeting = timeline(&[
            (Piece::Work, at(10, 0)),
            (Piece::Away, at(12, 10)),
            (Piece::Work, at(12, 30)),
        ]);
        assert_eq!(lunch_as_taken(lunch(), &meeting), lunch());
    }

    #[test]
    fn a_lock_joined_to_lunch_counts_in_full() {
        let early = timeline(&[
            (Piece::Work, at(11, 30)),
            (Piece::LockedAtLunch, at(12, 10)),
            (Piece::Work, at(12, 20)),
        ]);
        assert_eq!(lunch_as_taken(lunch(), &early), lunch_until(12, 10));
    }

    #[test]
    fn working_through_the_window_keeps_it() {
        let desk = timeline(&[(Piece::Work, at(13, 0))]);
        assert_eq!(lunch_as_taken(lunch(), &desk), lunch());
    }

    #[test]
    fn a_return_after_the_window_keeps_it() {
        let late = timeline(&[
            (Piece::Work, at(12, 0)),
            (Piece::Away, at(14, 10)),
            (Piece::Work, at(14, 30)),
        ]);
        assert_eq!(lunch_as_taken(lunch(), &late), lunch());
    }

    #[test]
    fn unknown_time_inside_the_window_is_not_lunch() {
        let gap = timeline(&[
            (Piece::Work, at(12, 0)),
            (Piece::Unknown, at(12, 40)),
            (Piece::Work, at(12, 50)),
        ]);
        assert_eq!(lunch_as_taken(lunch(), &gap), lunch());
    }

    #[test]
    fn unknown_time_neither_adds_to_the_break_nor_breaks_the_return() {
        let gap = timeline(&[
            (Piece::Work, at(12, 0)),
            (Piece::Away, at(12, 40)),
            (Piece::Work, at(12, 43)),
            (Piece::Unknown, at(12, 45)),
            (Piece::Work, at(12, 47)),
        ]);
        assert_eq!(lunch_as_taken(lunch(), &gap), lunch_until(12, 40));
    }

    #[test]
    fn two_short_breaks_with_work_between_are_not_lunch() {
        let coffees = timeline(&[
            (Piece::Work, at(12, 5)),
            (Piece::Away, at(12, 13)),
            (Piece::Work, at(12, 30)),
            (Piece::Away, at(12, 38)),
            (Piece::Work, at(12, 50)),
        ]);
        assert_eq!(lunch_as_taken(lunch(), &coffees), lunch());
    }
}
