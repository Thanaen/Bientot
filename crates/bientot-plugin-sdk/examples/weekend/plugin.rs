//! « Weekend »: counts the days to Friday, then the time to Friday's departure, and shows where
//! the week stands. Only the local date matters, which the snapshot gives as it is.

use bientot_plugin_sdk::api::view::MAX_DURATION_SECONDS;
use bientot_plugin_sdk::prelude::*;
use jiff::civil::Weekday;

#[derive(Default)]
struct Weekend;

fn show(day: &DaySnapshot) {
    let weekday = day.date.weekday();
    let first = match weekday {
        Weekday::Saturday | Weekday::Sunday => badge(text("weekend"), Tone::Success),
        _ if day.tracking == Tracking::NonWorkingDay => badge(text("day-off"), Tone::Success),
        Weekday::Friday => match day.departure_at_target {
            Some(departure) => row![text("friday-left"), countdown(departure)],
            None => badge(text("friday-done"), Tone::Success),
        },
        Weekday::Thursday => text("tomorrow").into(),
        other => text("days-left")
            .arg("count", 5 - other.to_monday_one_offset())
            .into(),
    };
    let leaving = (weekday == Weekday::Friday)
        .then_some(day.departure_at_target)
        .flatten()
        .map(|at| row![text("friday-at").muted(), time_at(at)]);
    let credit = day
        .week
        .credit
        .clamp(-MAX_DURATION_SECONDS, MAX_DURATION_SECONDS);
    let week = match credit {
        0 => text("even").muted().into(),
        c if c > 0 => row![text("ahead"), duration(c)],
        c => row![text("behind"), duration(-c)],
    };
    let _ = ui::show(card().line(first).line_if(leaving).line(week));
}

impl Plugin for Weekend {
    fn activate(&mut self, _: Language, _: ActivateReason) {
        if let Some(day) = day::current() {
            show(&day);
        }
    }

    fn day_refreshed(&mut self, day: &DaySnapshot) {
        show(day);
    }
}

bientot_plugin_sdk::export!(Weekend);

#[cfg(test)]
mod tests {
    use super::*;
    use bientot_plugin_sdk::testing::{TestHost, assert_reference, demo_day, demo_report};

    const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/weekend");

    #[test]
    fn every_demo_day() {
        for language in [Language::Fr, Language::En] {
            let report = demo_report::<Weekend>(DIR, language);
            assert_reference(format!("{DIR}/expected/{}.txt", language.tag()), &report);
        }
    }

    /// The on-track demo day, moved to another date of the same week.
    fn on(date: &str, credit: i64) -> DaySnapshot {
        let mut day = demo_day("OnTrack");
        day.date = date.parse().unwrap();
        day.week.credit = credit;
        day
    }

    #[test]
    fn a_friday_counts_down_to_the_departure() {
        let mut host = TestHost::load(DIR);
        let mut plugin = Weekend;
        host.refresh(&mut plugin, on("2026-09-04", 0)).unwrap();
        let card = host.render();
        assert!(card.contains("Weekend in ⏳1:00"), "{card}");
        assert!(card.contains("Leaving at 17:00"), "{card}");
    }

    #[test]
    fn the_week_shows_ahead_or_behind() {
        let mut host = TestHost::load(DIR);
        let mut plugin = Weekend;
        host.refresh(&mut plugin, on("2026-09-03", 5400)).unwrap();
        assert!(host.render().contains("Friday is tomorrow!"));
        assert!(host.render().contains("Ahead this week 1:30"));
        host.refresh(&mut plugin, on("2026-09-05", -600)).unwrap();
        assert!(host.render().contains("It’s the weekend"));
        assert!(host.render().contains("Behind this week 0:10"));
    }
}
