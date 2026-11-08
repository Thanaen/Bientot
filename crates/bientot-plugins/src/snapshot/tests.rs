use bientot_core::accounting::WorkdayAccounting;
use bientot_core::demo::{DemoEnvironment, DemoScenario};
use bientot_domain::configuration::Settings;
use bientot_domain::time::FixedTimeZoneProvider;
use bientot_store::adjustments::ManualAdjustmentRepository;
use bientot_store::sessions::SessionEventRepository;
use jiff::tz::TimeZone;

use super::*;

/// The snapshot of a demo scenario on a UTC machine with the default settings.
fn demo(scenario: DemoScenario, tracking: TrackingState, working_day: bool) -> DaySnapshot {
    let zones = FixedTimeZoneProvider::new("UTC", TimeZone::UTC);
    let directory = tempfile::tempdir().unwrap();
    let connection = bientot_store::database::open(&directory.path().join("worktime.db")).unwrap();
    bientot_store::database::migrate(&connection).unwrap();
    let demo = DemoEnvironment::new(&zones, scenario);
    let settings = Settings::default;
    let accounting = WorkdayAccounting::new(
        &demo,
        SessionEventRepository::new(&connection, &demo),
        ManualAdjustmentRepository::new(&connection),
        &demo,
        &zones,
        &settings,
    );
    let summary = demo.summary(&accounting).unwrap();
    let defaults = Settings::default();
    let at = |time: jiff::civil::Time| {
        summary
            .workday()
            .date
            .to_datetime(time)
            .to_zoned(TimeZone::UTC)
            .unwrap()
            .timestamp()
    };
    let lunch = UtcInterval::new(at(defaults.lunch_start), at(defaults.lunch_end)).unwrap();
    build(&DayInput {
        summary: &summary,
        tracking,
        working_day,
        lunch,
        minimum: defaults.minimum_effective_target,
        language: Language::Fr,
    })
}

#[test]
fn an_on_track_day() {
    let day = demo(DemoScenario::OnTrack, TrackingState::Running, true);
    assert_eq!(day.schema, SCHEMA);
    assert_eq!(day.tracking, Tracking::Running);
    assert_eq!(
        (day.counted, day.target, day.minimum),
        (6 * 3600, 7 * 3600, 6 * 3600)
    );
    assert_eq!(day.language, Language::Fr);
    assert_eq!(day.source, Source::Ok);
    // Six hours at 16:00: an hour left to the target, the minimum reached.
    assert_eq!(
        day.departure_at_target,
        Some(day.computed_at + SignedDuration::from_hours(1))
    );
    assert_eq!(day.departure_at_minimum, None);
    assert!(
        day.segments
            .iter()
            .all(|s| s.end <= day.computed_at && s.start < s.end)
    );
    let worked: i64 = day
        .segments
        .iter()
        .filter(|s| s.kind.is_work())
        .map(Segment::seconds)
        .sum::<i64>()
        + day
            .open_tail
            .filter(|t| t.kind.is_work())
            .map_or(0, |t| day.computed_at.as_second() - t.start.as_second());
    assert_eq!(worked, day.counted);
}

#[test]
fn a_morning_projects_both_departures_past_lunch() {
    let day = demo(DemoScenario::Morning, TrackingState::Running, true);
    let (at_target, at_minimum) = (
        day.departure_at_target.unwrap(),
        day.departure_at_minimum.unwrap(),
    );
    assert_eq!(
        at_target.duration_since(at_minimum),
        SignedDuration::from_hours(1)
    );
    let lunch = Settings::default();
    let without_lunch = day.computed_at + SignedDuration::from_secs(day.target - day.counted);
    assert_eq!(
        at_target.duration_since(without_lunch),
        lunch.lunch_end.duration_since(lunch.lunch_start)
    );
}

#[test]
fn a_complete_day_has_no_departure_left() {
    let day = demo(DemoScenario::Complete, TrackingState::Running, true);
    assert_eq!(day.counted, 8 * 3600);
    assert_eq!(day.departure_at_target, None);
    assert_eq!(day.departure_at_minimum, None);
}

#[test]
fn an_unavailable_source_has_no_forecast() {
    let day = demo(DemoScenario::Unavailable, TrackingState::Running, true);
    assert_eq!(day.source, Source::Degraded);
    assert_eq!(day.forecast, Forecast::Unavailable);
    assert_eq!(day.departure_at_target, None);
}

#[test]
fn tracking_and_working_days() {
    assert_eq!(
        demo(DemoScenario::OnTrack, TrackingState::Paused, true).tracking,
        Tracking::Paused
    );
    assert_eq!(
        demo(DemoScenario::OnTrack, TrackingState::Running, false).tracking,
        Tracking::NonWorkingDay
    );
}

#[test]
fn monday_credit_lowers_the_target_of_tuesday() {
    let day = demo(DemoScenario::Credit, TrackingState::Running, true);
    // The hour Monday earned is spent today: the target drops and nothing is left over.
    assert_eq!(day.target, 6 * 3600);
    assert_eq!(day.week.credit, 0);
}

#[test]
fn only_structural_changes_count() {
    let day = demo(DemoScenario::OnTrack, TrackingState::Running, true);
    let mut later = day.clone();
    later.computed_at += SignedDuration::from_mins(5);
    later.counted += 300;
    assert!(!is_structural(&day, &later), "counted time only grew");

    let mut moved = day.clone();
    moved.departure_at_target = moved
        .departure_at_target
        .map(|t| t + SignedDuration::from_secs(30));
    assert!(!is_structural(&day, &moved));
    moved.departure_at_target = day
        .departure_at_target
        .map(|t| t + SignedDuration::from_mins(1));
    assert!(is_structural(&day, &moved), "a departure moved by a minute");

    let mut reached = day.clone();
    reached.departure_at_target = None;
    assert!(is_structural(&day, &reached));

    let mut settled = day.clone();
    settled.segments.push(Segment {
        start: day.computed_at,
        end: day.computed_at + SignedDuration::from_mins(1),
        kind: SegmentKind::Away,
    });
    assert!(is_structural(&day, &settled));

    let mut paused = day.clone();
    paused.tracking = Tracking::Paused;
    assert!(is_structural(&day, &paused));
}

/// The SDK's test host reads the demo days from `bientot-plugin-sdk/demo-days.json`, which this
/// test writes with `BIENTOT_BLESS=1` and checks otherwise, so the file follows the accounting.
#[test]
fn the_sdk_demo_days_are_those_of_the_host() {
    let days: Vec<serde_json::Value> = DemoScenario::ALL
        .into_iter()
        .map(|scenario| {
            serde_json::json!({
                "scenario": format!("{scenario:?}"),
                "day": demo(scenario, TrackingState::Running, true),
            })
        })
        .collect();
    let mut actual = serde_json::to_string_pretty(&days).unwrap();
    actual.push('\n');
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../bientot-plugin-sdk/demo-days.json");
    if std::env::var_os("BIENTOT_BLESS").is_some() {
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    // Values, not text: a crate of the workspace turns on serde_json's `preserve_order`, which
    // changes the order of the keys but not the days.
    let expected: Option<Vec<serde_json::Value>> = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    assert!(
        expected.as_ref() == Some(&days),
        "{} is out of date: run `BIENTOT_BLESS=1 cargo test -p bientot-plugins sdk_demo_days`",
        path.display()
    );
}
