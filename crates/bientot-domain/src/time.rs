//! Clocks, time zones and the local workday.

use std::cell::Cell;

use jiff::civil::{Date, DateTime, Time, Weekday};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::{SignedDuration, Timestamp, ToSpan};

use crate::intervals::{EmptyInterval, UtcInterval};

pub trait Clock {
    fn now(&self) -> Timestamp;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// A clock that only moves when told to, forwards or backwards.
pub struct FrozenClock(Cell<Timestamp>);

impl FrozenClock {
    pub fn new(now: Timestamp) -> Self {
        Self(Cell::new(now))
    }

    pub fn advance(&self, by: SignedDuration) {
        self.0.set(self.0.get() + by);
    }

    pub fn set(&self, now: Timestamp) {
        self.0.set(now);
    }
}

impl Clock for FrozenClock {
    fn now(&self) -> Timestamp {
        self.0.get()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownTimeZone(pub String);

impl std::fmt::Display for UnknownTimeZone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "time zone {:?} is not available", self.0)
    }
}

impl std::error::Error for UnknownTimeZone {}

/// Time zones by the identifier stored with each workday. The identifiers are opaque here:
/// the platform decides their format (Windows identifiers in the existing database).
pub trait TimeZoneProvider {
    /// The current local zone and its identifier.
    fn local(&self) -> (String, TimeZone);

    fn get(&self, id: &str) -> Result<TimeZone, UnknownTimeZone>;
}

impl<T: TimeZoneProvider + ?Sized> TimeZoneProvider for &T {
    fn local(&self) -> (String, TimeZone) {
        (**self).local()
    }

    fn get(&self, id: &str) -> Result<TimeZone, UnknownTimeZone> {
        (**self).get(id)
    }
}

/// One zone under one identifier, for tests and demos.
pub struct FixedTimeZoneProvider {
    id: String,
    zone: TimeZone,
}

impl FixedTimeZoneProvider {
    pub fn new(id: impl Into<String>, zone: TimeZone) -> Self {
        Self {
            id: id.into(),
            zone,
        }
    }
}

impl TimeZoneProvider for FixedTimeZoneProvider {
    fn local(&self) -> (String, TimeZone) {
        (self.id.clone(), self.zone.clone())
    }

    fn get(&self, id: &str) -> Result<TimeZone, UnknownTimeZone> {
        if id == self.id {
            Ok(self.zone.clone())
        } else {
            Err(UnknownTimeZone(id.to_owned()))
        }
    }
}

/// A calendar day in the zone that was local when the day was first recorded.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LocalWorkday {
    pub date: Date,
    pub time_zone_id: String,
}

impl LocalWorkday {
    pub fn new(date: Date, time_zone_id: impl Into<String>) -> Self {
        Self {
            date,
            time_zone_id: time_zone_id.into(),
        }
    }

    pub fn is_working_day(&self) -> bool {
        !matches!(self.date.weekday(), Weekday::Saturday | Weekday::Sunday)
    }
}

/// The UTC span of a workday, 23 to 25 hours long across daylight saving changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkdayWindow {
    pub workday: LocalWorkday,
    pub start: Timestamp,
    pub end: Timestamp,
}

impl WorkdayWindow {
    pub fn duration(&self) -> SignedDuration {
        self.end.duration_since(self.start)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalendarError {
    UnknownTimeZone(UnknownTimeZone),
    EmptyInterval,
}

impl std::fmt::Display for CalendarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTimeZone(error) => error.fmt(f),
            Self::EmptyInterval => EmptyInterval.fmt(f),
        }
    }
}

impl std::error::Error for CalendarError {}

impl From<UnknownTimeZone> for CalendarError {
    fn from(error: UnknownTimeZone) -> Self {
        Self::UnknownTimeZone(error)
    }
}

impl From<EmptyInterval> for CalendarError {
    fn from(_: EmptyInterval) -> Self {
        Self::EmptyInterval
    }
}

pub struct WorkdayCalendar<P> {
    zones: P,
}

impl<P: TimeZoneProvider> WorkdayCalendar<P> {
    pub fn new(zones: P) -> Self {
        Self { zones }
    }

    pub fn zones(&self) -> &P {
        &self.zones
    }

    pub fn workday(&self, instant: Timestamp) -> LocalWorkday {
        let (id, zone) = self.zones.local();
        LocalWorkday::new(instant.to_zoned(zone).date(), id)
    }

    pub fn window(&self, workday: &LocalWorkday) -> Result<WorkdayWindow, CalendarError> {
        let next_day = workday
            .date
            .tomorrow()
            .expect("workdays are far from the calendar's end");
        Ok(WorkdayWindow {
            workday: workday.clone(),
            start: self.instant(workday, Time::midnight())?,
            end: self.instant(
                &LocalWorkday::new(next_day, workday.time_zone_id.clone()),
                Time::midnight(),
            )?,
        })
    }

    pub fn instant(&self, workday: &LocalWorkday, local: Time) -> Result<Timestamp, CalendarError> {
        let zone = self.zones.get(&workday.time_zone_id)?;
        Ok(resolve(&zone, workday.date.to_datetime(local)))
    }

    pub fn interval(
        &self,
        workday: &LocalWorkday,
        local_start: Time,
        local_end: Time,
    ) -> Result<UtcInterval, CalendarError> {
        if local_end <= local_start {
            return Err(CalendarError::EmptyInterval);
        }
        Ok(UtcInterval::new(
            self.instant(workday, local_start)?,
            self.instant(workday, local_end)?,
        )?)
    }
}

/// # Panics
///
/// When the result leaves the calendar, which workday dates never approach.
pub(crate) fn days_after(date: Date, days: i64) -> Date {
    date.checked_add(days.days())
        .expect("workdays are far from the calendar's end")
}

/// A local time skipped by a forward change moves to the first minute that exists; a
/// repeated local time takes its earlier occurrence. Same rule as the .NET app.
fn resolve(zone: &TimeZone, mut local: DateTime) -> Timestamp {
    loop {
        match zone.to_ambiguous_timestamp(local).offset() {
            AmbiguousOffset::Unambiguous { offset } => return to_timestamp(local, offset),
            AmbiguousOffset::Fold { before, after } => {
                return to_timestamp(local, before.max(after));
            }
            AmbiguousOffset::Gap { .. } => {
                local = local
                    .checked_add(1.minute())
                    .expect("a gap ends before the calendar does");
            }
        }
    }
}

fn to_timestamp(local: DateTime, offset: jiff::tz::Offset) -> Timestamp {
    offset
        .to_timestamp(local)
        .expect("workday times are far from the timestamp limits")
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::{date, time};
    use jiff::tz::Offset;

    const PARIS: &str = "Europe/Paris";

    fn paris() -> WorkdayCalendar<FixedTimeZoneProvider> {
        WorkdayCalendar::new(FixedTimeZoneProvider::new(
            PARIS,
            TimeZone::get(PARIS).unwrap(),
        ))
    }

    fn utc(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn workday_uses_the_local_date_across_utc_midnight() {
        let calendar = WorkdayCalendar::new(FixedTimeZoneProvider::new(
            "UTC+02-test",
            TimeZone::fixed(Offset::from_hours(2).unwrap()),
        ));

        let workday = calendar.workday(utc("2026-08-02T22:30:00Z"));

        assert_eq!(workday, LocalWorkday::new(date(2026, 8, 3), "UTC+02-test"));
        assert!(workday.is_working_day());
    }

    #[test]
    fn workday_window_reflects_daylight_saving_length() {
        let calendar = paris();
        for (day, hours) in [(date(2026, 3, 29), 23), (date(2026, 10, 25), 25)] {
            let window = calendar.window(&LocalWorkday::new(day, PARIS)).unwrap();
            assert_eq!(
                window.duration(),
                SignedDuration::from_hours(hours),
                "{day}"
            );
        }
    }

    #[test]
    fn local_time_in_the_spring_gap_moves_to_the_first_valid_instant() {
        let instant = paris()
            .instant(
                &LocalWorkday::new(date(2026, 3, 29), PARIS),
                time(2, 30, 0, 0),
            )
            .unwrap();

        assert_eq!(instant, utc("2026-03-29T01:00:00Z"));
    }

    #[test]
    fn repeated_local_time_uses_the_earlier_utc_occurrence() {
        let instant = paris()
            .instant(
                &LocalWorkday::new(date(2026, 10, 25), PARIS),
                time(2, 30, 0, 0),
            )
            .unwrap();

        assert_eq!(instant, utc("2026-10-25T00:30:00Z"));
    }

    #[test]
    fn local_interval_resolves_each_end_safely_across_daylight_saving() {
        let interval = paris()
            .interval(
                &LocalWorkday::new(date(2026, 3, 29), PARIS),
                time(1, 30, 0, 0),
                time(3, 30, 0, 0),
            )
            .unwrap();

        assert_eq!(
            interval,
            UtcInterval::new(utc("2026-03-29T00:30:00Z"), utc("2026-03-29T01:30:00Z")).unwrap()
        );
    }

    #[test]
    fn local_interval_rejects_an_end_before_its_start() {
        let result = paris().interval(
            &LocalWorkday::new(date(2026, 8, 3), PARIS),
            time(10, 0, 0, 0),
            time(9, 0, 0, 0),
        );

        assert_eq!(result, Err(CalendarError::EmptyInterval));
    }

    #[test]
    fn frozen_clock_moves_forwards_and_backwards() {
        let clock = FrozenClock::new(utc("2026-08-03T10:00:00Z"));

        clock.advance(SignedDuration::from_mins(30));
        assert_eq!(clock.now(), utc("2026-08-03T10:30:00Z"));

        clock.set(utc("2026-08-03T09:45:00Z"));
        assert_eq!(clock.now(), utc("2026-08-03T09:45:00Z"));
    }

    struct ChangingZones {
        current: Cell<&'static str>,
    }

    impl TimeZoneProvider for ChangingZones {
        fn local(&self) -> (String, TimeZone) {
            let id = self.current.get();
            (id.to_owned(), self.get(id).unwrap())
        }

        fn get(&self, id: &str) -> Result<TimeZone, UnknownTimeZone> {
            TimeZone::get(id).map_err(|_| UnknownTimeZone(id.to_owned()))
        }
    }

    #[test]
    fn a_recorded_workday_keeps_its_zone_after_the_local_zone_changes() {
        let calendar = WorkdayCalendar::new(ChangingZones {
            current: Cell::new(PARIS),
        });
        let workday = calendar.workday(utc("2026-08-03T08:00:00Z"));
        let original = calendar.window(&workday).unwrap();

        calendar.zones().current.set("UTC");

        assert_eq!(workday.time_zone_id, PARIS);
        assert_eq!(calendar.window(&workday).unwrap(), original);
    }

    #[test]
    fn weekend_is_not_a_working_day() {
        assert!(!LocalWorkday::new(date(2026, 8, 2), "UTC").is_working_day());
    }
}
