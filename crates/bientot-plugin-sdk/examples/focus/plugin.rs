//! « Focus »: the longest stretch of work of the day, the one under way, and how many lasted
//! 25 minutes or more. A short interruption (under two minutes) does not break a stretch.

use bientot_plugin_sdk::prelude::*;

/// An interruption shorter than this keeps the stretch going.
const GRACE: i64 = 2 * 60;
/// A stretch this long counts as a session.
const SESSION: i64 = 25 * 60;

#[derive(Default)]
struct Focus {
    detail: bool,
}

/// The stretches of work of a day, in seconds, and the start of the one under way.
#[derive(Debug, PartialEq)]
struct Stretches {
    lengths: Vec<i64>,
    current: Option<Timestamp>,
}

fn stretches(day: &DaySnapshot) -> Stretches {
    let mut pieces: Vec<(Timestamp, Timestamp, bool)> = day
        .segments
        .iter()
        .map(|s| (s.start, s.end, s.kind == SegmentKind::Work))
        .collect();
    if let Some(tail) = day.open_tail {
        pieces.push((tail.start, day.computed_at, tail.kind == SegmentKind::Work));
    }
    let mut lengths = Vec::new();
    let mut open: Option<(Timestamp, Timestamp)> = None;
    for (start, end, work) in pieces {
        let short = end.as_second() - start.as_second() < GRACE;
        match (&mut open, work) {
            (Some((_, last)), true) => *last = end,
            (None, true) => open = Some((start, end)),
            (Some(_), false) if short => {}
            (Some((first, last)), false) => {
                lengths.push(last.as_second() - first.as_second());
                open = None;
            }
            (None, false) => {}
        }
    }
    let running = day.open_tail.is_some_and(|t| t.kind == SegmentKind::Work);
    let current = open.filter(|_| running).map(|(first, _)| first);
    if let Some((first, last)) = open {
        lengths.push(last.as_second() - first.as_second());
    }
    Stretches { lengths, current }
}

impl Focus {
    fn show(&self, day: &DaySnapshot) {
        let stretches = stretches(day);
        let Some(&longest) = stretches.lengths.iter().max() else {
            let _ = ui::show(card().line(text("waiting").muted()));
            return;
        };
        let current = match stretches.current {
            Some(start) => row![text("current"), duration_since(0, start)],
            None => text("none").muted().into(),
        };
        let sessions = stretches.lengths.iter().filter(|&&l| l >= SESSION).count();
        let sessions = match sessions {
            0 => text("sessions-none"),
            1 => text("sessions-one"),
            n => text("sessions").arg("count", n),
        };
        let toggle = if self.detail { "less" } else { "more" };
        let _ = ui::show(
            card()
                .line(row![text("longest"), duration(longest)])
                .line(current)
                .line_if(self.detail.then(|| badge(sessions, Tone::Info)))
                .action(button("toggle-detail", text(toggle))),
        );
    }
}

impl Plugin for Focus {
    fn activate(&mut self, _: Language, _: ActivateReason) {
        if let Some(day) = day::current() {
            self.show(&day);
        }
    }

    fn day_refreshed(&mut self, day: &DaySnapshot) {
        self.show(day);
    }

    fn command(&mut self, command: &str, _: Option<&str>, _: CommandSource) {
        if command == "toggle-detail" {
            self.detail = !self.detail;
            if let Some(day) = day::current() {
                self.show(&day);
            }
        }
    }
}

bientot_plugin_sdk::export!(Focus);

#[cfg(test)]
mod tests {
    use super::*;
    use bientot_plugin_sdk::testing::{TestHost, assert_reference, demo_day, demo_report};

    const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/focus");

    #[test]
    fn every_demo_day() {
        for language in [Language::Fr, Language::En] {
            let report = demo_report::<Focus>(DIR, language);
            assert_reference(format!("{DIR}/expected/{}.txt", language.tag()), &report);
        }
    }

    #[test]
    fn the_detail_button_shows_the_sessions() {
        let mut host = TestHost::load(DIR);
        let mut plugin = Focus::default();
        host.refresh(&mut plugin, demo_day("OnTrack")).unwrap();
        assert_eq!(host.card().unwrap().lines.len(), 2);
        host.press(&mut plugin, 0).unwrap();
        let card = host.card().unwrap();
        assert_eq!(card.lines.len(), 3);
        assert_eq!(card.actions[0].label, "Less");
    }

    #[test]
    fn the_stretch_under_way_grows_with_the_clock() {
        let mut host = TestHost::load(DIR);
        let mut plugin = Focus::default();
        host.refresh(&mut plugin, demo_day("Morning")).unwrap();
        let before = host.render();
        host.advance(SignedDuration::from_mins(30));
        assert_ne!(before, host.render());
        assert!(
            host.render().contains("Under way for 2:30"),
            "{}",
            host.render()
        );
    }

    #[test]
    fn short_interruptions_do_not_break_a_stretch() {
        let day = demo_day("FragmentedDay");
        let found = stretches(&day);
        // 08:27 to 12:00 with blips of a few minutes at most: one stretch, then the afternoon.
        assert_eq!(found.lengths.len(), 3, "{found:?}");
        assert!(found.current.is_some());
    }
}
