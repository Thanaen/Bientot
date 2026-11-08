//! The day as plugins see it (data model, « DaySnapshot »): a hand-written type, never the
//! domain's types serialized as they are, so that the domain can change without breaking plugins.

use jiff::Timestamp;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use crate::text::Language;

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaySnapshot {
    pub schema: u32,
    pub computed_at: Timestamp,
    /// The IANA name of the user's zone, for example `Europe/Paris`.
    pub time_zone: String,
    pub date: Date,
    pub language: Language,
    pub tracking: Tracking,
    /// Seconds.
    pub target: i64,
    pub minimum: i64,
    pub counted: i64,
    /// When the counted time reaches the target, or `None` once reached or without a forecast.
    pub departure_at_target: Option<Timestamp>,
    pub departure_at_minimum: Option<Timestamp>,
    pub forecast: Forecast,
    /// Settled segments, oldest first.
    pub segments: Vec<Segment>,
    /// The segment still running, if any.
    pub open_tail: Option<OpenTail>,
    pub week: Week,
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tracking {
    Running,
    Paused,
    Finished,
    NonWorkingDay,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Forecast {
    /// The departure follows only from counted time and the schedule.
    Firm,
    /// The departure assumes the user keeps working as usual, for example after lunch.
    Estimated,
    Unavailable,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SegmentKind {
    Work,
    Break,
    Lunch,
    Away,
    Manual,
    Appointment,
    /// A kind the host has but does not name to plugins.
    Other,
    #[serde(other)]
    Unknown,
}

impl SegmentKind {
    /// Whether the segment counts as work time.
    pub fn is_work(self) -> bool {
        matches!(
            self,
            SegmentKind::Work | SegmentKind::Manual | SegmentKind::Appointment
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub start: Timestamp,
    pub end: Timestamp,
    pub kind: SegmentKind,
}

impl Segment {
    pub fn seconds(&self) -> i64 {
        self.end.as_second() - self.start.as_second()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenTail {
    pub start: Timestamp,
    pub kind: SegmentKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Week {
    /// Seconds counted this week, today included.
    pub counted: i64,
    pub target: i64,
    /// Seconds above (positive) or below the week's targets so far.
    pub credit: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    Ok,
    Degraded,
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/api-1.0/day-snapshot.json");

    #[test]
    fn the_api_1_0_fixture_still_reads() {
        let day: DaySnapshot = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(day.schema, SCHEMA);
        assert_eq!(day.date.to_string(), "2026-11-08");
        assert_eq!(day.tracking, Tracking::Running);
        assert_eq!(day.segments.len(), 3);
        assert_eq!(day.segments[0].seconds(), 5400);
        assert!(day.segments[0].kind.is_work());
        assert_eq!(day.open_tail.unwrap().kind, SegmentKind::Work);
        assert_eq!(day.week.credit, -1800);
    }

    #[test]
    fn later_values_read_as_unknown() {
        let later = FIXTURE
            .replace("\"running\"", "\"holiday\"")
            .replace("\"lunch\"", "\"training\"")
            .replace("\"firm\"", "\"guess\"")
            .replace("\"schema\": 1,", "\"schema\": 1, \"weather\": \"sunny\",");
        let day: DaySnapshot = serde_json::from_str(&later).unwrap();
        assert_eq!(day.tracking, Tracking::Unknown);
        assert_eq!(day.segments[2].kind, SegmentKind::Unknown);
        assert_eq!(day.forecast, Forecast::Unknown);
    }

    #[test]
    fn a_snapshot_round_trips() {
        let day: DaySnapshot = serde_json::from_str(FIXTURE).unwrap();
        let again: DaySnapshot =
            serde_json::from_str(&serde_json::to_string(&day).unwrap()).unwrap();
        assert_eq!(day, again);
    }
}
