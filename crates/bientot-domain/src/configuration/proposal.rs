//! The recurring appointment WorkTime proposes after the user corrected the same absence on
//! the same weekday again and again (issue #321): a pure reading of the corrections, which the
//! host gathers from the stored days.

use jiff::civil::{Date, Time, Weekday};
use jiff::{SignedDuration, ToSpan as _};

use super::{DismissedProposal, RecurringAppointment};

/// A day of the weekday looked at, as its stored summary shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedDay {
    pub date: Date,
    /// The user started the day: a day off counts neither for nor against a proposal.
    pub worked: bool,
    /// The spans the user's corrections count as work, in local time.
    pub corrected: Vec<(Time, Time)>,
}

/// A correction of one day that matches the proposed hours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub date: Date,
    pub start: Time,
    pub end: Time,
}

/// « Compter chaque jeudi… »: the appointment the corrections suggest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppointmentProposal {
    pub weekday: Weekday,
    /// The hours of the corrections, widened to the quarter hour.
    pub start: Time,
    pub end: Time,
    /// One week in two, the weeks of this day, when only those match.
    pub alternate_weeks_from: Option<Date>,
    /// The corrections that match, oldest first.
    pub occurrences: Vec<Occurrence>,
}

impl AppointmentProposal {
    /// How far back the corrections are read.
    pub const LOOKBACK_DAYS: i32 = 8 * 7;
    /// The days looked at, of which `REQUIRED` must match.
    pub const OBSERVED: usize = 4;
    pub const REQUIRED: usize = 3;
    /// Two corrections this close apart count as one.
    const JOIN_GAP: SignedDuration = SignedDuration::from_mins(5);
    const MINIMUM_LENGTH: SignedDuration = SignedDuration::from_mins(15);
    const MAXIMUM_LENGTH: SignedDuration = SignedDuration::from_hours(8);

    /// The appointment to propose on `weekday` from `days`, as of `today`, unless one of
    /// `appointments` on that weekday already covers its hours or the user turned it down in
    /// `dismissed`. The latest worked day must be one of the matches: the proposal answers the
    /// correction just made. Every week is tried first, then one week in two.
    pub fn find(
        weekday: Weekday,
        days: &[ObservedDay],
        today: Date,
        appointments: &[RecurringAppointment],
        dismissed: &[DismissedProposal],
    ) -> Option<Self> {
        // The editor of « Rendez-vous réguliers » offers the working days only.
        if matches!(weekday, Weekday::Saturday | Weekday::Sunday) {
            return None;
        }
        let earliest = today.checked_sub(Self::LOOKBACK_DAYS.days()).ok()?;
        let mut worked: Vec<&ObservedDay> = days
            .iter()
            .filter(|day| {
                day.worked
                    && day.date.weekday() == weekday
                    && earliest < day.date
                    && day.date <= today
            })
            .collect();
        worked.sort_by_key(|day| std::cmp::Reverse(day.date));
        worked.dedup_by_key(|day| day.date);
        let latest = worked.first()?.date;
        let every_week = worked.iter().copied().take(Self::OBSERVED);
        let alternate = worked
            .iter()
            .copied()
            .filter(|day| RecurringAppointment::same_alternate_weeks(day.date, latest))
            .take(Self::OBSERVED);
        let answers =
            |proposal: &Self| proposal.occurrences.last().map(|last| last.date) == Some(latest);
        let proposal = Self::matching(weekday, every_week, None)
            .filter(answers)
            .or_else(|| Self::matching(weekday, alternate, Some(latest)).filter(answers))?;
        let open = !appointments.iter().any(|appointment| {
            appointment.weekday == weekday
                && overlaps((appointment.start, appointment.end), proposal.hours())
        }) && !dismissed
            .iter()
            .any(|dismissal| dismissal.weekday == weekday && dismissal.holds(&proposal, today));
        open.then_some(proposal)
    }

    pub fn hours(&self) -> (Time, Time) {
        (self.start, self.end)
    }

    /// The refusal of this proposal, for good or until `until`.
    pub fn dismissal(&self, until: Option<Date>) -> DismissedProposal {
        DismissedProposal {
            weekday: self.weekday,
            start: self.start,
            end: self.end,
            until,
        }
    }

    /// The proposal `days` make, newest first, if `REQUIRED` of them match the hours of their
    /// longest corrections taken together.
    fn matching<'a>(
        weekday: Weekday,
        days: impl Iterator<Item = &'a ObservedDay>,
        alternate_weeks_from: Option<Date>,
    ) -> Option<Self> {
        let candidates: Vec<Occurrence> = days
            .filter_map(|day| {
                longest(&day.corrected).map(|(start, end)| Occurrence {
                    date: day.date,
                    start,
                    end,
                })
            })
            .collect();
        if candidates.len() < Self::REQUIRED {
            return None;
        }
        let start = floor_quarter(median(candidates.iter().map(|occurrence| occurrence.start)));
        let end = ceil_quarter(median(candidates.iter().map(|occurrence| occurrence.end)));
        let length = end.duration_since(start);
        if length < Self::MINIMUM_LENGTH || length > Self::MAXIMUM_LENGTH {
            return None;
        }
        let mut occurrences: Vec<Occurrence> = candidates
            .into_iter()
            .filter(|occurrence| similarity((occurrence.start, occurrence.end), (start, end)) > 0.5)
            .collect();
        if occurrences.len() < Self::REQUIRED {
            return None;
        }
        occurrences.reverse();
        Some(Self {
            weekday,
            start,
            end,
            alternate_weeks_from,
            occurrences,
        })
    }
}

/// The longest span of `spans` once those `JOIN_GAP` apart or less are joined.
fn longest(spans: &[(Time, Time)]) -> Option<(Time, Time)> {
    let mut sorted: Vec<(Time, Time)> = spans
        .iter()
        .copied()
        .filter(|(start, end)| start < end)
        .collect();
    sorted.sort();
    let mut joined: Vec<(Time, Time)> = Vec::new();
    for (start, end) in sorted {
        match joined.last_mut() {
            Some(last) if start.duration_since(last.1) <= AppointmentProposal::JOIN_GAP => {
                last.1 = last.1.max(end);
            }
            _ => joined.push((start, end)),
        }
    }
    joined
        .into_iter()
        .max_by_key(|(start, end)| end.duration_since(*start))
}

/// The median of `times`, the earlier of the two middle ones for an even count.
fn median(times: impl Iterator<Item = Time>) -> Time {
    let mut times: Vec<Time> = times.collect();
    times.sort();
    times[(times.len() - 1) / 2]
}

const QUARTER: i64 = 15 * 60;

fn floor_quarter(time: Time) -> Time {
    at_second(seconds(time) / QUARTER * QUARTER)
}

/// The quarter hour at or after `time`, the end of the day at most.
fn ceil_quarter(time: Time) -> Time {
    let exact = seconds(time) * 1_000_000_000 + i64::from(time.subsec_nanosecond());
    let quarter = QUARTER * 1_000_000_000;
    at_second((exact + quarter - 1) / quarter * QUARTER)
}

/// The time `second` seconds after midnight; the last instant of the day from midnight on.
fn at_second(second: i64) -> Time {
    Time::midnight()
        .checked_add(SignedDuration::from_secs(second))
        .ok()
        .filter(|_| second < 24 * 3600)
        .unwrap_or(Time::MAX)
}

fn seconds(time: Time) -> i64 {
    i64::from(time.hour()) * 3600 + i64::from(time.minute()) * 60 + i64::from(time.second())
}

/// Their common length over their combined length.
fn similarity(one: (Time, Time), other: (Time, Time)) -> f64 {
    let common = one.1.min(other.1).duration_since(one.0.max(other.0));
    if !common.is_positive() {
        return 0.;
    }
    let combined = one.1.max(other.1).duration_since(one.0.min(other.0));
    common.as_secs_f64() / combined.as_secs_f64()
}

fn overlaps(one: (Time, Time), other: (Time, Time)) -> bool {
    one.0 < other.1 && other.0 < one.1
}

#[cfg(test)]
mod tests;
